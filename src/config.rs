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
    #[serde(flatten)]
    pub config: EndpointTypeConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum EndpointTypeConfig {
    #[serde(rename = "openai")]
    OpenAI {
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        model: String,
        #[serde(default)]
        api_key_file: String,
        #[serde(default)]
        reasoning_effort: Option<String>,
        #[serde(default)]
        temperature: f32,
        #[serde(default)]
        no_context: bool,
    },
    #[serde(rename = "patchpal")]
    Patchpal {
        #[serde(default)]
        url: String,
    },
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file as YAML: {}", path.display()))?;

        if config.endpoints.is_empty() {
            return Err(anyhow::anyhow!(
                "No endpoints configured in config file: {}",
                path.display()
            ));
        }

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
        assert_eq!(config.endpoints.len(), 3);
        assert_eq!(config.endpoints[0].name, "Patchpal AI");
        assert_eq!(config.endpoints[1].name, "llama.cpp vulkan");
        assert_eq!(config.endpoints[2].name, "Gemini 2.5 pro");
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
