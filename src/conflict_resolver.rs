// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::api_client::{ApiClient, ApiRequest, ApiResponse};
use crate::config::{Config, EndpointConfig, EndpointTypeConfig};
use anyhow::Result;
use futures::future::select_all;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Conflict {
    pub file_path: String,
    pub local: String,
    pub base: String,
    pub remote: String,
    pub head_context: String,
    pub tail_context: String,
    pub start_line: usize,
    pub remote_start: usize,
    pub nr_head_context_lines: usize,
    pub nr_tail_context_lines: usize,
    pub marker_size: usize,
}

#[derive(Debug, Clone)]
pub struct ResolvedConflict {
    pub conflict: Conflict,
    pub resolved_version: String,
    pub model: String,
    pub duration: f64,
    pub total_tokens: Option<u64>,
}

pub struct ResolverErrors {
    pub errors: HashMap<String, usize>,
}

pub struct ConflictResolver<'a> {
    config: &'a Config,
    git_diff: Option<String>,
}

impl<'a> ConflictResolver<'a> {
    const DIFF_START: &'static str = "<|diff_start|>";
    const DIFF_END: &'static str = "<|diff_end|>";
    const PATCH_START: &'static str = "<|patch_start|>";
    const PATCH_END: &'static str = "<|patch_end|>";
    const CODE_START: &'static str = "<|code_start|>";
    const CODE_END: &'static str = "<|code_end|>";
    pub const PATCHED_CODE_START: &'static str = "<|patched_code_start|>";
    pub const PATCHED_CODE_END: &'static str = "<|patched_code_end|>";
    pub fn new(config: &'a Config, git_diff: Option<String>) -> Self {
        ConflictResolver {
            config,
            git_diff: Self::__git_diff(git_diff),
        }
    }

    /// Resolve all conflicts using AI
    pub async fn resolve_conflicts(
        self,
        conflicts: &[Conflict],
    ) -> Result<(Vec<ResolvedConflict>, ResolverErrors)> {
        let config = &self.config;
        let endpoints = config.get_all_endpoints();
        let mut resolved_conflicts = Vec::new();
        let mut resolver_errors = ResolverErrors {
            errors: HashMap::new(),
        };

        for (conflict_index, conflict) in conflicts.iter().enumerate() {
            let conflict_info = format!(
                "Resolving conflict {} of {} in {}:{}",
                conflict_index + 1,
                conflicts.len(),
                conflict.file_path,
                conflict.start_line
            );
            println!("{}", conflict_info);
            log::info!("{}", conflict_info);

            // Create the prompt for AI resolution
            let prompt = self.create_prompt(conflict);
            let patch = self.create_patch(conflict);
            let code = self.create_code(conflict);
            let message = self.create_message(&patch, &code);
            let git_diff = self.create_git_diff(conflict);

            // Try to resolve with all endpoints in parallel
            let mut futures = Vec::new();
            for (endpoint_index, endpoint) in endpoints.iter().enumerate() {
                let client = ApiClient::new(endpoint.clone());
                let name = endpoint.name.clone();
                let api_request = ApiRequest {
                    prompt: prompt.clone(),
                    message: message.clone(),
                    patch: patch.clone(),
                    code: code.clone(),
                    endpoint: endpoint.clone(),
                    git_diff: git_diff.clone(),
                };
                let handle = tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    let result = client.query(&api_request).await;
                    let duration = start.elapsed();
                    (result, duration, name, endpoint_index)
                });
                futures.push(handle);
            }

            let mut results = Vec::new();
            while !futures.is_empty() {
                let (result, _, remaining) = select_all(futures).await;
                futures = remaining;
                match result {
                    Ok((result, duration, name, endpoint_index)) => {
                        let duration = duration.as_secs_f64();
                        println!(
                            " - {} completed in {:.2} s{}",
                            name,
                            duration,
                            result
                                .as_ref()
                                .map(|r| r.total_tokens.map(|t| format!(
                                    " - tokens {} - {:.2} t/s",
                                    t,
                                    t as f64 / duration
                                )))
                                .unwrap_or_default()
                                .unwrap_or_default()
                        );
                        results.push((result, duration, endpoint_index))
                    }
                    Err(e) => return Err(anyhow::anyhow!("Task failed: {}", e)),
                }
            }

            self.process_results(
                &mut resolved_conflicts,
                &mut resolver_errors,
                results,
                conflict,
                endpoints,
            );
        }

        Ok((resolved_conflicts, resolver_errors))
    }

    fn get_model_name_z(
        &self,
        endpoints: &[EndpointConfig],
        i: usize,
        y: usize,
        z: usize,
        dups: usize,
    ) -> String {
        let endpoint = &endpoints[i];
        let variant = match &endpoint.config {
            EndpointTypeConfig::OpenAI { params, .. } => {
                if let Some(params) = params {
                    if let Some(param) = params.get(y) {
                        match param.variant.as_deref() {
                            Some(variant) => format!("{} ({})", endpoint.name, variant),
                            None => endpoint.name.clone(),
                        }
                    } else {
                        assert!(y == 0); // When we have params, we expect to be able to index into them
                        endpoint.name.clone()
                    }
                } else {
                    // No params defined, use endpoint name directly
                    endpoint.name.clone()
                }
            }
            EndpointTypeConfig::Patchpal { .. } => {
                format!("{} #{}", endpoint.name, y)
            }
        };
        if z > 0 {
            format!("{} #{}", variant, z + 1 - dups)
        } else {
            variant
        }
    }

    fn get_model_name(&self, endpoints: &[EndpointConfig], i: usize, y: usize) -> String {
        self.get_model_name_z(endpoints, i, y, 0, 0)
    }

    fn process_results(
        &self,
        resolved_conflicts: &mut Vec<ResolvedConflict>,
        resolver_errors: &mut ResolverErrors,
        results: Vec<(Result<ApiResponse>, f64, usize)>,
        conflict: &Conflict,
        endpoints: &[EndpointConfig],
    ) {
        // Validate that the content starts with head_context and ends with tail_context
        for result in results {
            let endpoint_index = result.2;
            let duration = result.1;
            let result = match result.0 {
                Ok(r) => r,
                Err(e) => {
                    log::warn!(
                        "Skipping {} due to error: {}",
                        endpoints[endpoint_index].name,
                        e
                    );
                    continue;
                }
            };

            let total_tokens = result.total_tokens;
            let resolved = self.parse_response(result);

            assert!(!resolved.is_empty());
            for (y, resolved) in resolved.iter().enumerate() {
                let resolved_strings = match resolved {
                    Ok(resolved_strings) => resolved_strings,
                    Err(e) => {
                        let model = self.get_model_name(endpoints, endpoint_index, y);
                        log::warn!("Skipping {} - {}", model, e);
                        *resolver_errors.errors.entry(model).or_insert(0) += 1;
                        continue;
                    }
                };

                assert!(!resolved_strings.is_empty());
                let mut dups = 0;
                let mut seen_resolved = std::collections::HashMap::new();
                for (z, resolved_string) in resolved_strings.iter().enumerate() {
                    let model = self.get_model_name_z(endpoints, endpoint_index, y, z, dups);
                    if !resolved_string.starts_with(&conflict.head_context) {
                        log::warn!("Skipping {} - doesn't start with head context", model);
                        *resolver_errors.errors.entry(model).or_insert(0) += 1;

                        continue;
                    }
                    if !resolved_string.ends_with(&conflict.tail_context) {
                        log::warn!("Skipping {} - doesn't end with tail context", model);
                        *resolver_errors.errors.entry(model).or_insert(0) += 1;
                        continue;
                    }
                    //reduce resolved to the range between head_context and tail_context
                    let resolved_content = resolved_string[conflict.head_context.len()
                        ..resolved_string.len() - conflict.tail_context.len()]
                        .to_string();
                    if !resolved_content.is_empty() && !resolved_content.ends_with('\n') {
                        log::warn!(
                            "Skipping {} - resolved content is not newline terminated",
                            model
                        );
                        *resolver_errors.errors.entry(model).or_insert(0) += 1;
                        continue;
                    }
                    // Check if this resolved_content is already in the results
                    let key = (endpoint_index, resolved_content.clone());
                    if seen_resolved.contains_key(&key) {
                        log::debug!("Skipping {} - duplicate resolved conflict", model);
                        dups += 1;
                        continue;
                    }
                    seen_resolved.insert(key, model.clone());

                    resolved_conflicts.push(ResolvedConflict {
                        conflict: conflict.clone(),
                        resolved_version: resolved_content,
                        model,
                        duration,
                        total_tokens,
                    });
                }
            }
        }
    }

    /// Create a prompt for the AI to resolve the conflict
    fn __git_diff(git_diff: Option<String>) -> Option<String> {
        git_diff.map(|s| {
            format!(
                r#"The PATCH originates from the DIFF between {diff_start}{diff_end}.

{diff_start}
{s}{diff_end}
"#,
                diff_start = Self::DIFF_START,
                diff_end = Self::DIFF_END,
            )
        })
    }

    fn create_git_diff(&self, conflict: &Conflict) -> Option<String> {
        if let Some(git_diff) = &self.git_diff
            && git_diff.contains(&conflict.file_path)
        {
            return Some(git_diff.clone());
        }
        None
    }

    /// Create a prompt for the AI to resolve the conflict
    fn create_prompt(&self, conflict: &Conflict) -> String {
        format!(
            r#"Apply the PATCH between {patch_start}{patch_end} to the CODE between {code_start}{code_end}.

FINALLY write the final PATCHED CODE between {patched_code_start}{patched_code_end} instead of markdown fences.

Rewrite the {nr_head_context_lines} lines after {code_start} and the {nr_tail_context_lines} lines before {code_end} exactly the same, including all empty lines."#,
            patch_start = Self::PATCH_START,
            patch_end = Self::PATCH_END,
            code_start = Self::CODE_START,
            code_end = Self::CODE_END,
            patched_code_start = Self::PATCHED_CODE_START,
            patched_code_end = Self::PATCHED_CODE_END,
            nr_head_context_lines = conflict.nr_head_context_lines,
            nr_tail_context_lines = conflict.nr_tail_context_lines
        )
    }

    fn create_message(&self, patch: &String, code: &String) -> String {
        format!(
            r#"{patch_start}
{patch}{patch_end}

{code_start}
{code}{code_end}
"#,
            patch_start = Self::PATCH_START,
            patch = patch,
            patch_end = Self::PATCH_END,
            code_start = Self::CODE_START,
            code = code,
            code_end = Self::CODE_END,
        )
    }

    fn create_patch(&self, conflict: &Conflict) -> String {
        let base = format!(
            "{}{}{}",
            conflict.head_context, conflict.base, conflict.tail_context
        );
        let remote = format!(
            "{}{}{}",
            conflict.head_context, conflict.remote, conflict.tail_context
        );
        use imara_diff::{Algorithm, BasicLineDiffPrinter, Diff, InternedInput, UnifiedDiffConfig};
        let input = InternedInput::new(&base[..], &remote[..]);
        let mut diff = Diff::compute(Algorithm::Histogram, &input);
        diff.postprocess_lines(&input);
        let mut config = UnifiedDiffConfig::default();
        config.context_len(
            conflict
                .nr_head_context_lines
                .max(conflict.nr_tail_context_lines) as u32,
        );
        diff.unified_diff(&BasicLineDiffPrinter(&input.interner), config, &input)
            .to_string()
    }

    fn create_code(&self, conflict: &Conflict) -> String {
        format!(
            "{}{}{}",
            conflict.head_context, conflict.local, conflict.tail_context
        )
    }

    /// Parse the API response into 3 solutions
    fn parse_response(&self, response: ApiResponse) -> Vec<Result<Vec<String>>> {
        let start_marker = format!("{}\n", Self::PATCHED_CODE_START);
        let end_marker = Self::PATCHED_CODE_END;
        let mut all_results = Vec::new();

        for response in response.responses {
            let response = match response {
                Ok(response) => response,
                Err(e) => {
                    all_results.push(Err(e));
                    continue;
                }
            };

            log::info!("Response:\n{}", response);

            let mut results = Vec::new();
            let mut err: Option<Result<Vec<String>, anyhow::Error>> = None;
            let mut start = 0;

            while let Some(start_pos) = response[start..].find(&start_marker) {
                let start_pos = start + start_pos;
                let end_pos = response[start_pos..].find(end_marker);
                if end_pos.is_none() {
                    err = Some(Err(anyhow::anyhow!(
                        "Invalid format: missing {}",
                        Self::PATCHED_CODE_END
                    )));
                    break;
                }

                let end_pos = start_pos + end_pos.unwrap();
                if start_pos > end_pos {
                    err = Some(Err(anyhow::anyhow!(
                        "Invalid format: {} appears after {}",
                        Self::PATCHED_CODE_START,
                        Self::PATCHED_CODE_END
                    )));
                    break;
                }

                let content_start = start_pos + start_marker.len();
                let content_end = end_pos;

                let content = &response[content_start..content_end];
                results.push(content.to_string());

                start = end_pos + end_marker.len();
            }

            if results.is_empty() {
                let err = match err {
                    Some(err) => err,
                    None => Err(anyhow::anyhow!("No code blocks found in response")),
                };
                all_results.push(err);
            } else {
                all_results.push(Ok(results));
            }
        }
        all_results
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
