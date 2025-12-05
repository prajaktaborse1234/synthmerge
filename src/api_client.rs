// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::config::{EndpointConfig, EndpointJson, EndpointTypeConfig, EndpointVariants};
use crate::conflict_resolver::ConflictResolver;
use crate::prob;
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
    pub training: String,
}

#[derive(Clone, Debug)]
pub struct ApiResponseEntry {
    pub response: String,
    pub logprob: Option<f64>,
    pub total_tokens: Option<u64>,
    pub duration: f64,
}

macro_rules! get_context_field {
    ($endpoint_context:expr, $variant_context:expr, $field:ident) => {{
        let endpoint_value = $endpoint_context.as_ref().and_then(|ctx| ctx.$field);
        let variant_value = $variant_context.as_ref().and_then(|ctx| ctx.$field);
        endpoint_value.or(variant_value).unwrap_or(false)
    }};
}

pub type ApiResponse = Vec<Result<ApiResponseEntry>>;

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
            .timeout(Duration::from_millis(endpoint.timeout))
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

        let (variants, no_chat) = match &request.endpoint.config {
            EndpointTypeConfig::OpenAI {
                variants, no_chat, ..
            } => (variants, no_chat),
            _ => panic!("cannot happen"),
        };

        // Handle EndpointVariants - if None, create a single entry with no parameters
        let variants_list = match variants {
            Some(variants) => variants,
            None => &vec![EndpointVariants::default()],
        };

        let mut responses = Vec::new();

        for variant in variants_list {
            let chat = self.create_chat(request, variant);
            let mut payload = if !*no_chat {
                let mut payload = serde_json::json!({
                            "messages": [],
                });
                let messages = payload["messages"].as_array_mut().unwrap();
                for (i, msg) in chat.iter().enumerate().filter(|(_, s)| s.is_some()) {
                    let role = if i == 0 {
                        "system"
                    } else if i % 2 == 1 {
                        "user"
                    } else {
                        "assistant"
                    };
                    messages.push(serde_json::json!({
                        "role": role,
                        "content": msg
                    }));
                }
                payload
            } else {
                let prompt = chat
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.is_some())
                    //.filter(|(i, s)| s.is_some() && (*i == 0 || i % 2 == 1))
                    .map(|(_, s)| s.as_ref().unwrap().clone())
                    .collect::<Vec<_>>()
                    .join("\n\n")
                    + "\n\n";
                serde_json::json!({
                    "prompt": prompt
                })
            };

            self.apply_parameters(&mut payload, &request.endpoint.json)?;
            self.apply_parameters(&mut payload, &variant.json)?;

            let result = self
                .retry_request(
                    &request.endpoint.url,
                    headers.clone(),
                    &payload,
                    |response_text: &str, duration: f64| -> Result<ApiResponseEntry> {
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
                            .or_else(|| {
                                json_response
                                    .get("choices")
                                    .and_then(|choices| choices.get(0))
                                    .and_then(|choice| choice.get("text"))
                            })
                            .and_then(|content| content.as_str())
                            .with_context(|| {
                                log::warn!(
                                    "Failed to extract content from response:\n{}",
                                    serde_json::to_string_pretty(&json_response).unwrap()
                                );
                                "Failed to extract content from response"
                            })?;

                        let logprob = prob::logprob(&json_response);

                        let total_tokens = json_response
                            .get("usage")
                            .and_then(|usage| usage.get("total_tokens"))
                            .and_then(|tokens| tokens.as_u64());

                        Ok(ApiResponseEntry {
                            response: content.to_string(),
                            logprob,
                            total_tokens,
                            duration,
                        })
                    },
                )
                .await;
            responses.push(result);
        }

        Ok(responses)
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

        for variant in variants_list {
            let chat = self.create_chat(request, variant);

            let mut payload = serde_json::json!({
                "system": chat[0],
                "messages": [],
            });
            let messages = payload["messages"].as_array_mut().unwrap();
            for (i, msg) in chat[1..].iter().enumerate().filter(|(_, s)| s.is_some()) {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                messages.push(serde_json::json!({
                    "role": role,
                    "content": [{"type": "text", "text": msg}]
                }));
            }

            self.apply_parameters(&mut payload, &request.endpoint.json)?;
            self.apply_parameters(&mut payload, &variant.json)?;

            let result = self
                .retry_request(
                    &request.endpoint.url,
                    headers.clone(),
                    &payload,
                    |response_text: &str, duration: f64| -> Result<ApiResponseEntry> {
                        // Parse JSON response to extract the content
                        let json_response: serde_json::Value = serde_json::from_str(response_text)
                            .with_context(|| {
                                log::warn!("Failed to parse JSON response:\n{}", response_text);
                                "Failed to parse JSON response"
                            })?;

                        let content = json_response
                            .get("content")
                            .and_then(|choices| choices.get(0))
                            .and_then(|choice| {
                                if let Some(type_val) = choice.get("type").and_then(|v| v.as_str())
                                {
                                    if type_val == "text" {
                                        choice.get("text").and_then(|text| text.as_str())
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .with_context(|| {
                                log::warn!(
                                    "Failed to extract content from response:\n{}",
                                    serde_json::to_string_pretty(&json_response).unwrap()
                                );
                                "Failed to extract content from response"
                            })?;

                        let logprob = prob::logprob(&json_response);

                        let total_tokens = json_response
                            .get("usage")
                            .and_then(|usage| usage.get("input_tokens"))
                            .and_then(|tokens| tokens.as_u64())
                            .map(|input_tokens| {
                                input_tokens
                                    + json_response
                                        .get("usage")
                                        .and_then(|usage| usage.get("output_tokens"))
                                        .and_then(|tokens| tokens.as_u64())
                                        .unwrap_or(0)
                            });

                        Ok(ApiResponseEntry {
                            response: content.to_string(),
                            logprob,
                            total_tokens,
                            duration,
                        })
                    },
                )
                .await;
            responses.push(result)
        }

        Ok(responses)
    }

    fn create_chat(&self, request: &ApiRequest, variant: &EndpointVariants) -> Vec<Option<String>> {
        let mut chat = Vec::new();

        if get_context_field!(
            &request.endpoint.context,
            &variant.context,
            with_system_message
        ) {
            let mut prompt = format!("{}\n\n{}", request.prompt, request.training);
            if let Some(git_diff) = &request.git_diff
                && !get_context_field!(&request.endpoint.context, &variant.context, no_diff)
            {
                prompt = format!("{}\n\n{}", prompt, git_diff)
            }
            chat.push(Some(prompt));
            chat.push(Some(request.message.to_string()));
        } else {
            chat.push(Some(request.prompt.clone()));
            let mut msg = request.message.clone();
            if let Some(git_diff) = &request.git_diff
                && !get_context_field!(&request.endpoint.context, &variant.context, no_diff)
            {
                msg = format!("{}\n\n{}", git_diff, msg)
            }
            msg = format!("{}\n\n{}", request.training, msg);
            chat.push(Some(msg));
        }

        log::debug!(
            "{}",
            chat.iter()
                .enumerate()
                .filter(|(_, s)| s.is_some())
                .map(|(i, s)| format!("Chat[{}]:\n{}", i, s.as_ref().unwrap()))
                .collect::<Vec<_>>()
                .join("\n")
        );
        chat
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

        let response_handler = |response_text: &str, duration: f64| -> Result<ApiResponse> {
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
                    (
                        v.get(0)
                            .and_then(|v| v.as_str())
                            .context("Failed to extract patched code from patchpal response"),
                        v.get(1)
                            .and_then(|v| v.as_f64())
                            .context("Failed to extract logprobs from patchpal response"),
                    )
                })
                .map(|s| -> Result<ApiResponseEntry> {
                    Ok(ApiResponseEntry {
                        response: format!(
                            "{}\n{}{}",
                            ConflictResolver::PATCHED_CODE_START,
                            s.0?,
                            ConflictResolver::PATCHED_CODE_END
                        ),
                        logprob: s.1.ok(),
                        total_tokens: None,
                        duration,
                    })
                })
                .collect();
            if responses.iter().any(Result::is_err) {
                log::warn!(
                    "Failed to extract content from patchpal response:\n{}",
                    serde_json::to_string_pretty(&json_response).unwrap()
                );
            }

            Ok(responses)
        };

        self.retry_request(&request.endpoint.url, headers, &payload, response_handler)
            .await
    }

    async fn retry_request<F, R>(
        &self,
        url: &str,
        headers: reqwest::header::HeaderMap,
        payload: &serde_json::Value,
        response_handler: F,
    ) -> Result<R>
    where
        F: Fn(&str, f64) -> Result<R>,
    {
        let start = std::time::Instant::now();
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
                    let duration = start.elapsed().as_secs_f64();
                    log::trace!(
                        "Response JSON ({}):\n{}",
                        self.endpoint.name,
                        serde_json::to_string_pretty(
                            &serde_json::from_str(&response_text)
                                .unwrap_or(serde_json::Value::String(response_text.clone()))
                        )
                        .unwrap_or(response_text.clone())
                    );

                    match response_handler(&response_text, duration) {
                        Ok(api_response) => {
                            self.apply_wait().await;
                            return Ok(api_response);
                        }
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
                        self.apply_wait().await;
                        return Err(e.into());
                    }
                    self.apply_delay(&mut delay, max_delay, &e).await;
                    last_error = Some(e.into());
                }
            }
        }
        Err(last_error.context("Failed to send request after retries")?)
    }

    async fn apply_wait(&self) {
        if self.endpoint.wait != 0 {
            log::trace!("Waiting {}ms before next request", self.endpoint.wait);
            tokio::time::sleep(Duration::from_millis(self.endpoint.wait)).await;
        }
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
