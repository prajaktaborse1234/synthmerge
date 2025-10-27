// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::config::{EndpointConfig, EndpointTypeConfig};
use crate::conflict_resolver::ConflictResolver;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    pub prompt: String,
    pub message: String,
    pub patch: String,
    pub code: String,
    pub endpoint: EndpointConfig,
    pub git_diff: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse {
    pub response: String,
    pub total_tokens: Option<u64>,
}

pub struct ApiClient {
    endpoint: EndpointConfig,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(endpoint: EndpointConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        ApiClient { endpoint, client }
    }

    /// Query the AI endpoint with the given prompt
    pub async fn query(&self, api_request: &ApiRequest) -> Result<ApiResponse> {
        let response = match &self.endpoint.config {
            EndpointTypeConfig::OpenAI { api_key_file, .. } => {
                let api_key = if let Some(key_file) = api_key_file {
                    std::fs::read_to_string(shellexpand::full(key_file)?.as_ref())
                        .context("Failed to read API key file")?
                        .trim()
                        .to_string()
                } else {
                    String::new()
                };
                self.query_openai(&api_key, api_request).await
            }
            EndpointTypeConfig::Patchpal { .. } => self.query_patchpal(api_request).await,
        }?;

        Ok(response)
    }

    async fn query_openai(&self, api_key: &str, request: &ApiRequest) -> Result<ApiResponse> {
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

        let (model, reasoning_effort, temperature, no_context) = match &request.endpoint.config {
            EndpointTypeConfig::OpenAI {
                model,
                reasoning_effort,
                temperature,
                no_context,
                ..
            } => (model, reasoning_effort, temperature, no_context),
            _ => panic!("cannot happen"),
        };
        let prompt = if let Some(git_diff) = &request.git_diff
            && !no_context.unwrap_or(false)
        {
            format!("{}\n\n{}", request.prompt, git_diff)
        } else {
            request.prompt.clone()
        };
        log::debug!("Prompt:\n{}", prompt);
        let mut payload = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": prompt},
                {"role": "user", "content": request.message},
            ],
        });
        if let Some(reasoning_effort) = reasoning_effort {
            payload["reasoning_effort"] = serde_json::Value::String(reasoning_effort.to_string());
        }
        if let Some(temperature) = temperature {
            let temperature = serde_json::Number::from_f64(*temperature);
            let temperature =
                serde_json::Value::Number(temperature.expect("Temperature value is required"));
            payload["temperature"] = temperature;
        }
        log::debug!("Request raw: {}", payload);

        let response = self
            .client
            .post(request.endpoint.url.clone())
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to OpenAI API")?;

        let response_text = response.text().await.context("Failed to read response")?;
        log::debug!("Response raw:\n{}", response_text);

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

        let total_tokens = json_response
            .get("usage")
            .and_then(|usage| usage.get("total_tokens"))
            .and_then(|tokens| tokens.as_u64());

        Ok(ApiResponse {
            response: content.to_string(),
            total_tokens,
        })
    }

    async fn query_patchpal(&self, request: &ApiRequest) -> Result<ApiResponse> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let payload = serde_json::json!({"jsonrpc": "2.0",
					 "method": "inference",
					 "params" : {"patch" : request.patch,
						     "code" : request.code}});
        let response = self
            .client
            .post(request.endpoint.url.clone())
            .headers(headers.clone())
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to patchpal API")?;

        let response_text = response.text().await.context("Failed to read response")?;
        log::debug!("Response raw:\n{}", response_text);

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
                Ok(format!(
                    "{}\n{}{}",
                    ConflictResolver::PATCHED_CODE_START,
                    s?,
                    ConflictResolver::PATCHED_CODE_END
                ))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");

        Ok(ApiResponse {
            response: content.to_string(),
            total_tokens: None,
        })
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
