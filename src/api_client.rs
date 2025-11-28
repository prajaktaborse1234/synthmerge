// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::config::{EndpointConfig, EndpointJson, EndpointTypeConfig, EndpointVariants};
use crate::conflict_resolver::{ConflictResolver, Training};
use anyhow::{Context, Result};
use reqwest::Certificate;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

#[derive(Debug)]
pub struct ApiRequest {
    pub prompt: String,
    pub message: String,
    pub patch: String,
    pub code: String,
    pub endpoint: EndpointConfig,
    pub git_diff: Option<String>,
    pub training: Training,
}

#[derive(Debug)]
pub struct ApiResponse {
    pub responses: Vec<Result<String>>,
    pub total_tokens: Option<u64>,
}

pub struct ApiClient {
    endpoint: EndpointConfig,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(endpoint: EndpointConfig) -> Self {
        let client = Self::create_client(&endpoint);

        ApiClient {
            endpoint,
            client: client.expect("Failed to create client"),
        }
    }

    fn create_client(endpoint: &EndpointConfig) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(endpoint.timeout))
            .tcp_keepalive(Duration::from_secs(10));

        // Add root certificate if specified
        if let Some(cert_path) = &endpoint.root_certificate_pem {
            let cert_path = shellexpand::full(cert_path)?;
            let mut buf = Vec::new();
            File::open(cert_path.as_ref())
                .and_then(|mut file| file.read_to_end(&mut buf))
                .map_err(|e| {
                    anyhow::anyhow!("Failed to read certificate file {}: {}", cert_path, e)
                })?;
            let cert = Certificate::from_pem(&buf).map_err(|e| {
                anyhow::anyhow!("Failed to parse certificate from {}: {}", cert_path, e)
            })?;
            builder = builder.add_root_certificate(cert);
            log::trace!("Root certificate loaded successfully from {}", cert_path);
        }

        builder
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build client: {}", e))
    }

    /// Query the AI endpoint with the given prompt
    pub async fn query(&self, api_request: &ApiRequest) -> Result<ApiResponse> {
        let response = match &self.endpoint.config {
            EndpointTypeConfig::OpenAI { .. } => self.query_openai(api_request).await,
            EndpointTypeConfig::Patchpal { .. } => self.query_patchpal(api_request).await,
            EndpointTypeConfig::Anthropic { .. } => self.query_anthropic(api_request).await,
        }?;

        Ok(response)
    }

    async fn read_api_key(&self, api_key_file: &String) -> Result<String> {
        Ok(
            std::fs::read_to_string(shellexpand::full(api_key_file)?.as_ref())
                .context("Failed to read API key file")?
                .trim()
                .to_string(),
        )
    }

    async fn create_headers(
        &self,
        api_key_file: &Option<String>,
    ) -> Result<reqwest::header::HeaderMap> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        if let Some(api_key_file) = api_key_file {
            // Only add the Authorization header if an API key file is specified
            let api_key = self.read_api_key(api_key_file).await?;
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .context("Invalid API key")?,
            );
        }
        Ok(headers)
    }

    async fn query_openai(&self, request: &ApiRequest) -> Result<ApiResponse> {
        let headers = self.create_headers(&request.endpoint.api_key_file).await?;

        let variants = match &request.endpoint.config {
            EndpointTypeConfig::OpenAI { variants, .. } => variants,
            _ => panic!("cannot happen"),
        };

        // Handle EndpointVariants - if None, create a single entry with no parameters
        let variants_list = match variants {
            Some(variants) => variants,
            None => &vec![EndpointVariants::default()],
        };

        let mut responses = Vec::new();
        let mut total_tokens: Option<u64> = None;

        for variant in variants_list {
            let prompt = self.build_prompt(request, variant);

            let mut payload = serde_json::json!({
                "messages": [
                    {"role": "system", "content": prompt},
                    {"role": "user", "content": request.message},
                ],
            });

            self.apply_parameters(&mut payload, &request.endpoint.json)?;
            self.apply_parameters(&mut payload, &variant.json)?;

            let result = self
                .retry_request(
                    &request.endpoint.url,
                    headers.clone(),
                    &payload,
                    |response_text: &str| -> Result<ApiResponse> {
                        // Parse JSON response to extract the content
                        let json_response: serde_json::Value = serde_json::from_str(response_text)
                            .with_context(|| {
                                log::warn!("Failed to parse JSON response:\n{}", response_text);
                                "Failed to parse JSON response"
                            })?;

                        let content = json_response
                            .get("choices")
                            .and_then(|choices| choices.get(0))
                            .and_then(|choice| choice.get("message"))
                            .and_then(|message| message.get("content"))
                            .and_then(|content| content.as_str())
                            .with_context(|| {
                                log::warn!(
                                    "Failed to extract content from response:\n{}",
                                    serde_json::to_string_pretty(&json_response).unwrap()
                                );
                                "Failed to extract content from response"
                            })?;

                        let total_tokens = json_response
                            .get("usage")
                            .and_then(|usage| usage.get("total_tokens"))
                            .and_then(|tokens| tokens.as_u64());

                        Ok(ApiResponse {
                            responses: vec![Ok(content.to_string())],
                            total_tokens,
                        })
                    },
                )
                .await;
            match result {
                Ok(result) => {
                    if let Some(param_total_tokens) = result.total_tokens {
                        total_tokens = match total_tokens {
                            Some(tokens) => Some(tokens + param_total_tokens),
                            None => Some(param_total_tokens),
                        }
                    }

                    responses.extend(result.responses);
                }
                Err(e) => responses.push(Err(e)),
            }
        }

        Ok(ApiResponse {
            responses,
            total_tokens,
        })
    }

    async fn query_anthropic(&self, request: &ApiRequest) -> Result<ApiResponse> {
        let variants = match &request.endpoint.config {
            EndpointTypeConfig::Anthropic { variants, .. } => variants,
            _ => panic!("cannot happen"),
        };

        let headers = self.create_headers(&request.endpoint.api_key_file).await?;

        // Handle EndpointVariants - if None, create a single entry with no parameters
        let variants_list = match variants {
            Some(variants) => variants,
            None => &vec![EndpointVariants::default()],
        };

        let mut responses = Vec::new();
        let mut total_tokens: Option<u64> = None;

        for variant in variants_list {
            let prompt = self.build_prompt(request, variant);

            let mut payload = serde_json::json!({
                "messages": [
                    {"role": "user", "content": [{"type": "text",
                      "text": format!("{}\n\n{}", prompt, request.message)}]}, ],
            });

            self.apply_parameters(&mut payload, &request.endpoint.json)?;
            self.apply_parameters(&mut payload, &variant.json)?;

            let result = self
                .retry_request(
                    &request.endpoint.url,
                    headers.clone(),
                    &payload,
                    |response_text: &str| -> Result<ApiResponse> {
                        // Parse JSON response to extract the content
                        let json_response: serde_json::Value = serde_json::from_str(response_text)
                            .with_context(|| {
                                log::warn!("Failed to parse JSON response:\n{}", response_text);
                                "Failed to parse JSON response"
                            })?;

                        let content = json_response
                            .get("content")
                            .and_then(|choices| choices.get(0))
                            .and_then(|choice| choice.get("text"))
                            .and_then(|content| content.as_str())
                            .with_context(|| {
                                log::warn!(
                                    "Failed to extract content from response:\n{}",
                                    serde_json::to_string_pretty(&json_response).unwrap()
                                );
                                "Failed to extract content from response"
                            })?;

                        let total_tokens = json_response
                            .get("usage")
                            .and_then(|usage| usage.get("output_tokens"))
                            .and_then(|tokens| tokens.as_u64());

                        Ok(ApiResponse {
                            responses: vec![Ok(content.to_string())],
                            total_tokens,
                        })
                    },
                )
                .await;
            match result {
                Ok(result) => {
                    if let Some(param_total_tokens) = result.total_tokens {
                        total_tokens = match total_tokens {
                            Some(tokens) => Some(tokens + param_total_tokens),
                            None => Some(param_total_tokens),
                        }
                    }

                    responses.extend(result.responses);
                }
                Err(e) => responses.push(Err(e)),
            }
        }

        Ok(ApiResponse {
            responses,
            total_tokens,
        })
    }

    fn build_prompt(&self, request: &ApiRequest, variant: &EndpointVariants) -> String {
        let context = if request.endpoint.context.is_some() {
            &request.endpoint.context
        } else {
            &variant.context
        };

        let mut prompt = format!("{}\n\n{}", request.prompt, request.training);
        if let Some(git_diff) = &request.git_diff
            && !context.as_ref().map(|x| x.no_diff).unwrap_or(false)
        {
            prompt = format!("{}\n\n{}", prompt, git_diff)
        }

        log::debug!("Prompt:\n{}", prompt);
        log::info!("Message:\n{}", request.message);

        prompt
    }

    fn apply_parameters(
        &self,
        payload: &mut serde_json::Value,
        json: &Option<EndpointJson>,
    ) -> Result<()> {
        // Apply parameters from EndpointVariants JSON
        if let Some(json) = json {
            for (key, value) in &json.json {
                if payload.get(key).is_some() {
                    return Err(anyhow::anyhow!("Key {} already present in payload", key));
                }
                payload[key] = value.clone();
            }
        }
        Ok(())
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

        let response_handler = |response_text: &str| -> Result<ApiResponse> {
            // Try to parse as JSON and extract content
            let json_response: serde_json::Value = serde_json::from_str(response_text)
                .with_context(|| {
                    log::warn!("Failed to parse JSON response:\n{}", response_text);
                    "Failed to parse JSON response"
                })?;
            if json_response.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
                log::warn!(
                    "Invalid patchpal jsonrpc version:\n{}",
                    serde_json::to_string_pretty(&json_response).unwrap()
                );
                return Err(anyhow::anyhow!("Invalid patchpal jsonrpc version"));
            }
            let responses: Vec<_> = json_response
                .get("result")
                .and_then(|v| v.as_array())
                .context("Failed to extract content from patchpal response")?
                .iter()
                .map(|v| {
                    v.get(0)
                        .and_then(|v| v.as_str())
                        .context("Failed to extract string from patchpal response")
                })
                .map(|s| -> Result<String> {
                    Ok(format!(
                        "{}\n{}{}",
                        ConflictResolver::PATCHED_CODE_START,
                        s?,
                        ConflictResolver::PATCHED_CODE_END
                    ))
                })
                .collect();
            if responses.iter().any(Result::is_err) {
                log::warn!(
                    "Failed to extract content from patchpal response:\n{}",
                    serde_json::to_string_pretty(&json_response).unwrap()
                );
            }

            Ok(ApiResponse {
                responses,
                total_tokens: None,
            })
        };

        self.retry_request(&request.endpoint.url, headers, &payload, response_handler)
            .await
    }

    async fn retry_request<F>(
        &self,
        url: &str,
        headers: reqwest::header::HeaderMap,
        payload: &serde_json::Value,
        response_handler: F,
    ) -> Result<ApiResponse>
    where
        F: Fn(&str) -> Result<ApiResponse>,
    {
        let mut last_error = None;
        let mut delay = Duration::from_millis(self.endpoint.delay);
        let max_delay = Duration::from_millis(self.endpoint.max_delay);

        log::trace!(
            "Request JSON ({}):\n{}",
            self.endpoint.name,
            serde_json::to_string_pretty(payload).unwrap()
        );

        for _ in 0..self.endpoint.retries {
            let response = self
                .client
                .post(url.to_string())
                .headers(headers.clone())
                .json(payload)
                .send()
                .await;

            match response {
                Ok(response) => {
                    let response_text = match response.text().await {
                        Ok(text) => text,
                        Err(e) => {
                            self.apply_delay(&mut delay, max_delay, &e).await;
                            last_error = Some(e.into());
                            continue;
                        }
                    };
                    log::trace!(
                        "Response JSON ({}):\n{}",
                        self.endpoint.name,
                        serde_json::to_string_pretty(
                            &serde_json::from_str(&response_text)
                                .unwrap_or(serde_json::Value::String(response_text.clone()))
                        )
                        .unwrap_or(response_text.clone())
                    );

                    match response_handler(&response_text) {
                        Ok(api_response) => return Ok(api_response),
                        Err(e) => {
                            self.apply_delay(&mut delay, max_delay, &e).await;
                            last_error = Some(e);
                        }
                    }
                }
                Err(e) => {
                    if e.is_timeout() {
                        // Don't retry on timeout errors or it may waste energy
                        log::warn!(
                            "Timeout error for endpoint {}. Consider increasing the timeout.",
                            self.endpoint.name
                        );
                        return Err(e.into());
                    }
                    self.apply_delay(&mut delay, max_delay, &e).await;
                    last_error = Some(e.into());
                }
            }
        }
        Err(last_error.context("Failed to send request after retries")?)
    }

    async fn apply_delay<E>(&self, delay: &mut Duration, max_delay: Duration, error: &E)
    where
        E: std::fmt::Display + 'static,
    {
        log::warn!(
            "Retrying endpoint {} after error: {}",
            self.endpoint.name,
            error
        );
        tokio::time::sleep(*delay).await;
        *delay = std::cmp::min(*delay * 2, max_delay);
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
