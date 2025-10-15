// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::config::EndpointConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    pub prompt: String,
    pub message: String,
    pub patch: String,
    pub code: String,
    pub reasoning_effort: Option<String>,
    pub temperature: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse {
    pub response: String,
}

pub struct ApiClient {
    config: EndpointConfig,
    client: reqwest::Client,
    verbose: bool,
}

impl ApiClient {
    pub fn new(config: EndpointConfig, verbose: bool) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        ApiClient {
            config,
            client,
            verbose,
        }
    }

    /// Query the AI endpoint with the given prompt
    pub async fn query(
        &self,
        prompt: &str,
        message: &str,
        patch: &str,
        code: &str,
    ) -> Result<String> {
        let api_request = ApiRequest {
            prompt: prompt.to_string(),
            message: message.to_string(),
            patch: patch.to_string(),
            code: code.to_string(),
            reasoning_effort: self.config.config.reasoning_effort().cloned(),
            temperature: self.config.config.temperature(),
        };

        let response = match &self.config.config {
            crate::config::EndpointTypeConfig::OpenAI {
                url, api_key_file, ..
            } => {
                let api_key = std::fs::read_to_string(shellexpand::full(api_key_file)?.as_ref())
                    .context("Failed to read API key file")?;
                self.query_openai(
                    url.as_ref().map(|s| s.as_str()).unwrap_or(""),
                    api_key.trim(),
                    &api_request,
                )
                .await
            }
            crate::config::EndpointTypeConfig::Patchpal { url, .. } => {
                self.query_patchpal(url, &api_request).await
            }
        }?;

        Ok(response)
    }

    async fn query_openai(
        &self,
        api_url: &str,
        api_key: &str,
        request: &ApiRequest,
    ) -> Result<String> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
                .context("Invalid API key")?,
        );

        let mut payload = serde_json::json!({
            "model": self.config.config.model(),
            "messages": [
                {"role": "system", "content": request.prompt},
                {"role": "user", "content": request.message},
            ],
            "temperature": request.temperature,
        });
        if let Some(reasoning_effort) = &request.reasoning_effort {
            payload["reasoning_effort"] = reasoning_effort.clone().into();
        }
        if self.verbose {
            println!("headers: {:?}", headers);
            println!("json: {}", payload);
        }

        let response = self
            .client
            .post(api_url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to OpenAI API")?;

        let response_text = response.text().await.context("Failed to read response")?;
        if self.verbose {
            println!("Response:\n{}", response_text);
        }

        // Parse JSON response to extract the content
        let json_response: serde_json::Value =
            serde_json::from_str(&response_text).context("Failed to parse JSON response")?;

        let content = json_response
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_str())
            .context("Failed to extract content from response")?;

        Ok(content.to_string())
    }

    async fn query_patchpal(&self, url: &str, request: &ApiRequest) -> Result<String> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let payload = serde_json::json!(
           {"jsonrpc": "2.0",
        "method": "inference",
            "params" : {
        "patch" : request.patch,
        "code" : request.code
            }});

        let response = self
            .client
            .post(url)
            .headers(headers.clone())
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to patchpal API")?;

        let response_text = response.text().await.context("Failed to read response")?;
        if self.verbose {
            println!("Response:\n{}", response_text);
        }

        // Try to parse as JSON and extract content
        let json_response: serde_json::Value =
            serde_json::from_str(&response_text).context("Failed to parse JSON response")?;
        if json_response.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
            return Err(anyhow::anyhow!("Invalid patchpal jsonrpc version"));
        }
        let content = json_response
            .get("result")
            .and_then(|v| v.as_array())
            .context("Failed to extract content from patchpal response")?
            .iter()
            .map(|v| {
                v.get(0)
                    .and_then(|v| v.as_str())
                    .context("Failed to extract string from patchpal response")
            })
            .map(|s| -> Result<String, anyhow::Error> {
                Ok(format!("{}{}{}", "<|code_start|>\n", s?, "<|code_end|>"))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        if self.verbose {
            println!("Content:\n{}", content);
        }
        Ok(content.to_string())
    }
}

// Extension trait to get config values
impl crate::config::EndpointTypeConfig {
    fn model(&self) -> &str {
        match self {
            crate::config::EndpointTypeConfig::OpenAI { model, .. } => model,
            _ => "",
        }
    }

    fn reasoning_effort(&self) -> Option<&String> {
        match self {
            crate::config::EndpointTypeConfig::OpenAI {
                reasoning_effort, ..
            } => reasoning_effort.as_ref(),
            _ => None,
        }
    }

    fn temperature(&self) -> f32 {
        match self {
            crate::config::EndpointTypeConfig::OpenAI { temperature, .. } => *temperature,
            _ => 0.,
        }
    }
}
