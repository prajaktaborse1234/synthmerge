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
    #[serde(flatten)]
    pub config: EndpointTypeConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum EndpointTypeConfig {
    #[serde(rename = "openai")]
    OpenAI {
        #[serde(default)]
        model: String,
        #[serde(default)]
        api_key_file: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<String>,
        #[serde(default)]
        temperature: Option<f64>,
        #[serde(default)]
        no_context: Option<bool>,
    },
    #[serde(rename = "patchpal")]
    Patchpal {},
}

impl Config {
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
        for endpoint in &mut config.endpoints {
            endpoint.name = endpoint.name.trim().to_string();
            endpoint.url = endpoint.url.trim().to_string();
        }

        // Check that each endpoint has required fields
        for (i, endpoint) in config.endpoints.iter().enumerate() {
            if endpoint.name.is_empty() {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has empty name",
                    i,
                    path.display()
                ));
            }
            if endpoint.url.trim().is_empty() {
                return Err(anyhow::anyhow!(
                    "Endpoint {} in config file {} has empty url",
                    i,
                    path.display()
                ));
            }
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

    pub fn get_all_endpoints(&self) -> &[EndpointConfig] {
        &self.endpoints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        let config_yaml = include_str!("../synthmerge.yaml");
        let config: Config = serde_yaml::from_str(config_yaml).unwrap();
        assert_eq!(config.endpoints.len(), 4);
        assert_eq!(config.endpoints[0].name, "Patchpal AI");
        assert_eq!(config.endpoints[1].name, "llama.cpp vulkan");
        assert_eq!(config.endpoints[2].name, "llama.cpp vulkan no_context");
        assert_eq!(config.endpoints[3].name, "Gemini 2.5 pro");
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
