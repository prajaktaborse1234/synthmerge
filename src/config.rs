// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    endpoints: Vec<EndpointConfig>,
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
    #[serde(default)]
    pub wait: u64,
    pub root_certificate_pem: Option<String>,
    pub api_key_file: Box<Option<String>>,
    pub context: Option<EndpointContext>,
    pub json: Option<EndpointJson>,
    #[serde(flatten)]
    pub config: EndpointTypeConfig,
}

fn default_timeout() -> u64 {
    600
}

fn default_retries() -> u32 {
    100
}

fn default_delay() -> u64 {
    10000
}

fn default_max_delay() -> u64 {
    600000
}

macro_rules! check_conflicting_context_fields {
    ($endpoint_context:expr, $variant_context:expr, $variant_name:expr, $endpoint_index:expr, $path:expr, $j:expr, $($field:ident),*) => {
        $(
            if $endpoint_context.$field.is_some() && $variant_context.$field.is_some() {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has conflicting context configuration in variant {} at index {}: {} is defined in both endpoint and variant",
                    $endpoint_index,
                    $path.display(),
                    $variant_name,
                    $j,
                    stringify!($field)
                ));
            }
        )*
    };
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum EndpointTypeConfig {
    #[serde(rename = "openai")]
    OpenAI {
        variants: Option<Vec<EndpointVariants>>,
        #[serde(default)]
        no_chat: bool, // false: /v1/chat/completions true /v1/completions
    },
    #[serde(rename = "anthropic")]
    Anthropic {
        variants: Option<Vec<EndpointVariants>>,
    },
    #[serde(rename = "patchpal")]
    Patchpal {},
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EndpointVariants {
    pub name: Box<Option<String>>,
    pub context: Option<EndpointContext>,
    pub json: Option<EndpointJson>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EndpointContext {
    #[serde(default)]
    pub with_system_message: Option<bool>,
    #[serde(default)]
    pub no_diff: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EndpointJson {
    #[serde(flatten)]
    pub json: std::collections::HashMap<String, serde_json::Value>,
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
            Self::validate_endpoint(endpoint, i, path)?;
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
            Self::trim_variants_whitespace(&mut endpoint.config);
        }
    }

    fn trim_variants_whitespace(config: &mut EndpointTypeConfig) {
        if let EndpointTypeConfig::OpenAI { variants, .. }
        | EndpointTypeConfig::Anthropic { variants, .. } = config
            && let Some(variant_list) = variants
        {
            for variant in variant_list {
                if let Some(variant) = &mut *variant.name {
                    *variant = variant.trim().to_string();
                }
            }
        }
    }

    fn validate_endpoint(endpoint: &EndpointConfig, index: usize, path: &Path) -> Result<()> {
        if let EndpointTypeConfig::OpenAI { variants, .. }
        | EndpointTypeConfig::Anthropic { variants, .. } = &endpoint.config
        {
            // Check that all variant names are unique
            Self::validate_variants(variants, index, path, &endpoint.json, &endpoint.context)?;
        }
        Ok(())
    }

    fn validate_variants(
        variants_list: &Option<Vec<EndpointVariants>>,
        endpoint_index: usize,
        path: &Path,
        endpoint_json: &Option<EndpointJson>,
        endpoint_context: &Option<EndpointContext>,
    ) -> Result<()> {
        if let Some(variants_list) = variants_list {
            // Check that all variant names are unique
            let mut seen_variants = std::collections::HashSet::new();
            // Collect all keys from endpoint.json
            let mut seen_keys = std::collections::HashSet::new();
            if let Some(endpoint_json) = endpoint_json {
                for key in endpoint_json.json.keys() {
                    if !seen_keys.insert(key) {
                        return Err(anyhow::anyhow!(
                            "Endpoint {} in config file {} has duplicate key '{}' in endpoint.json",
                            endpoint_index,
                            path.display(),
                            key
                        ));
                    }
                }
            }
            for (j, variant) in variants_list.iter().enumerate() {
                let variant_name = if let Some(v) = &*variant.name {
                    v.to_string()
                } else {
                    "".to_string()
                };
                if variant_name
                    .chars()
                    .any(|c| Self::FORBIDDEN_CHARS.contains(c))
                {
                    return Err(anyhow::anyhow!(
                        "Endpoint {} in config file {} has invalid variant name '{}' at index {} contains {} chars",
                        endpoint_index,
                        path.display(),
                        variant_name,
                        j,
                        Self::FORBIDDEN_CHARS
                    ));
                }
                if !seen_variants.insert(variant_name) {
                    return Err(anyhow::anyhow!(
                        "Endpoint {} in config file {} has duplicate variant name '{}' at index {}",
                        endpoint_index,
                        path.display(),
                        variant.name.as_deref().unwrap_or(""),
                        j
                    ));
                }
                // Check for duplicate keys between endpoint.json and variant.json
                if let Some(variant_json) = &variant.json {
                    for key in variant_json.json.keys() {
                        if !seen_keys.insert(key) {
                            return Err(anyhow::anyhow!(
                                "Endpoint {} in config file {} has duplicate key '{}' in variant {} at index {}",
                                endpoint_index,
                                path.display(),
                                key,
                                variant.name.clone().unwrap_or("\"\"".to_string()),
                                j
                            ));
                        }
                    }
                }
                // Check if endpoint.context.is_some() and variant.context.is_some(),
                // then a field can be Some only in either the endpoint.context or the
                // variant.context but not in both
                if let (Some(endpoint_context), Some(variant_context)) =
                    (&endpoint_context, &variant.context)
                {
                    check_conflicting_context_fields!(
                        endpoint_context,
                        variant_context,
                        variant.name.clone().unwrap_or("\"\"".to_string()),
                        endpoint_index,
                        path,
                        j,
                        with_system_message,
                        no_diff
                    );
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
        assert_eq!(config.endpoints.len(), 7);
        assert_eq!(config.endpoints[0].name, "Claude Sonnet 4.0");
        assert_eq!(config.endpoints[1].name, "Patchpal AI");
        assert_eq!(config.endpoints[2].name, "Gemini 3 pro preview");
        assert_eq!(config.endpoints[3].name, "Gemini 2.5 pro");
        assert_eq!(config.endpoints[4].name, "llama.cpp vulkan minimal");
        assert_eq!(config.endpoints[5].name, "llama.cpp vulkan");
        assert_eq!(config.endpoints[6].name, "llama.cpp vulkan no_chat");
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
