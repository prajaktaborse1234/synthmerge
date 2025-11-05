// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub endpoints: Vec<EndpointConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EndpointConfig {
    pub name: String,
    pub url: String,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_delay")]
    pub delay: u64,
    #[serde(default = "default_max_delay")]
    pub max_delay: u64,
    #[serde(flatten)]
    pub config: EndpointTypeConfig,
}

fn default_timeout() -> u64 {
    600
}

fn default_retries() -> u32 {
    10
}

fn default_delay() -> u64 {
    1000
}

fn default_max_delay() -> u64 {
    600000
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum EndpointTypeConfig {
    #[serde(rename = "openai")]
    OpenAI {
        #[serde(default)]
        model: String,
        #[serde(default)]
        api_key_file: Box<Option<String>>,
        #[serde(default)]
        params: Option<Vec<OpenAIParams>>,
    },
    #[serde(rename = "patchpal")]
    Patchpal {},
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OpenAIParams {
    #[serde(default)]
    pub variant: Box<Option<String>>,
    #[serde(default)]
    pub no_context: Option<bool>,
    #[serde(default)]
    pub reasoning_effort: Box<Option<String>>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_k: Option<i32>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub min_p: Option<f64>,
}

impl Config {
    const FORBIDDEN_CHARS: &str = "()|,";

    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let mut config: Config = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file as YAML: {}", path.display()))?;

        if config.endpoints.is_empty() {
            return Err(anyhow::anyhow!(
                "No endpoints configured in config file: {}",
                path.display()
            ));
        }

        // Trim whitespace from endpoint
        Self::trim_endpoint_whitespace(&mut config.endpoints);

        // Check that each endpoint has required fields
        for (i, endpoint) in config.endpoints.iter().enumerate() {
            if endpoint.name.is_empty() {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has empty name",
                    i,
                    path.display()
                ));
            }
            if endpoint
                .name
                .chars()
                .any(|c| Self::FORBIDDEN_CHARS.contains(c))
            {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has invalid name '{}' contains {} chars",
                    i,
                    path.display(),
                    endpoint.name,
                    Self::FORBIDDEN_CHARS
                ));
            }
            if endpoint.url.is_empty() {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has empty url",
                    i,
                    path.display()
                ));
            }

            // Validate OpenAI endpoint configuration
            Self::validate_openai_endpoint(endpoint, i, path)?;
        }

        let mut seen_names = std::collections::HashSet::new();
        for (i, endpoint) in config.endpoints.iter().enumerate() {
            if !seen_names.insert(&endpoint.name) {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has duplicate name '{}'",
                    i,
                    path.display(),
                    endpoint.name
                ));
            }
        }
        log::debug!("{:?}", config);

        Ok(config)
    }

    fn trim_endpoint_whitespace(endpoints: &mut [EndpointConfig]) {
        for endpoint in endpoints {
            endpoint.name = endpoint.name.trim().to_string();
            endpoint.url = endpoint.url.trim().to_string();
            if let EndpointTypeConfig::OpenAI { params, .. } = &mut endpoint.config
                && let Some(param_list) = params
            {
                for param in param_list {
                    if let Some(variant) = &mut *param.variant {
                        *variant = variant.trim().to_string();
                    }
                }
            }
        }
    }

    fn validate_openai_endpoint(
        endpoint: &EndpointConfig,
        index: usize,
        path: &Path,
    ) -> Result<()> {
        if let EndpointTypeConfig::OpenAI { params, .. } = &endpoint.config {
            if params.as_ref().is_some_and(|p| p.is_empty()) {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has empty OpenAI params",
                    index,
                    path.display()
                ));
            }
            // Check that all variant names are unique
            let mut seen_variants = std::collections::HashSet::new();
            if let Some(param_list) = params {
                for (j, param) in param_list.iter().enumerate() {
                    let variant = if let Some(v) = &*param.variant {
                        v.to_string()
                    } else {
                        "".to_string()
                    };
                    if param_list.len() != 1 && variant.is_empty() {
                        return Err(anyhow::anyhow!(
                            "Endpoint {} in config file {} has empty variant name at index {}",
                            index,
                            path.display(),
                            j
                        ));
                    }
                    if variant.chars().any(|c| Self::FORBIDDEN_CHARS.contains(c)) {
                        return Err(anyhow::anyhow!(
                            "Endpoint {} in config file {} has invalid variant name '{}' at index {} contains {} chars",
                            index,
                            path.display(),
                            variant,
                            j,
                            Self::FORBIDDEN_CHARS
                        ));
                    }
                    if !seen_variants.insert(variant) {
                        return Err(anyhow::anyhow!(
                            "Endpoint {} in config file {} has duplicate variant name '{}' at index {}",
                            index,
                            path.display(),
                            param.variant.as_deref().unwrap_or(""),
                            j
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get_all_endpoints(&self) -> &[EndpointConfig] {
        &self.endpoints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        let config_yaml = include_str!(concat!("../", env!("CARGO_PKG_NAME"), ".yaml"));
        let config: Config = serde_yaml::from_str(config_yaml).unwrap();
        assert_eq!(config.endpoints.len(), 4);
        assert_eq!(config.endpoints[0].name, "Patchpal AI");
        assert_eq!(config.endpoints[1].name, "llama.cpp vulkan simple");
        assert_eq!(config.endpoints[2].name, "llama.cpp vulkan");
        assert_eq!(config.endpoints[3].name, "Gemini 2.5 pro");
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
