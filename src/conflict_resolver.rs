// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::api_client::{ApiClient, ApiRequest, ApiResponse};
use crate::config::{Config, EndpointConfig, EndpointTypeConfig};
use crate::git_utils::ContextLines;
use crate::prob;
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
    pub remote_end: usize,
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
    pub logprob: Option<f64>,
}

pub struct ResolverErrors {
    pub errors: HashMap<String, usize>,
}

pub struct ConflictResolver<'a> {
    context_lines: ContextLines,
    config: &'a Config,
    git_diff: Option<String>,
    training: String,
    bench: bool,
}

impl<'a> ConflictResolver<'a> {
    const DIFF_START: &'static str = "<|diff|>";
    const DIFF_END: &'static str = "<|/diff|>";
    const PATCH_START: &'static str = "<|patch|>";
    const PATCH_END: &'static str = "<|/patch|>";
    const CODE_START: &'static str = "<|code|>";
    const CODE_END: &'static str = "<|/code|>";
    pub const PATCHED_CODE_START: &'static str = "<|patched_code|>";
    pub const PATCHED_CODE_END: &'static str = "<|/patched_code|>";
    pub fn new(
        context_lines: ContextLines,
        config: &'a Config,
        git_diff: Option<String>,
        bench: bool,
    ) -> Self {
        ConflictResolver {
            context_lines,
            config,
            git_diff: Self::__git_diff(git_diff),
            training: Self::create_training(),
            bench,
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
            if !self.bench {
                let conflict_info = format!(
                    "Resolving conflict {} of {} in {}:{}",
                    conflict_index + 1,
                    conflicts.len(),
                    conflict.file_path,
                    conflict.start_line
                );
                println!("{}", conflict_info);
                log::info!("{}", conflict_info);
            }

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
                    training: self.training.clone(),
                    message: message.clone(),
                    patch: patch.clone(),
                    code: code.clone(),
                    git_diff: git_diff.clone(),
                };
                let handle = tokio::spawn(async move {
                    let result = client.query(&api_request).await;
                    (result, name, endpoint_index)
                });
                futures.push(handle);
            }

            let mut results = Vec::new();
            while !futures.is_empty() {
                let (result, _, remaining) = select_all(futures).await;
                futures = remaining;
                match result {
                    Ok((result, name, endpoint_index)) => {
                        println!(
                            " - {}{}",
                            name,
                            self.print_api_response(&result, endpoints, endpoint_index)
                        );
                        results.push((result, endpoint_index))
                    }
                    Err(e) => return Err(anyhow::anyhow!("Task failed: {}", e)),
                }
            }

            self.process_results(
                &mut resolved_conflicts,
                &mut resolver_errors,
                &results,
                conflict,
                endpoints,
            );
        }

        Ok((resolved_conflicts, resolver_errors))
    }

    fn print_api_response(
        &self,
        api_response: &Result<ApiResponse>,
        endpoints: &[EndpointConfig],
        endpoint_index: usize,
    ) -> String {
        api_response
            .as_ref()
            .map(|r| {
                let mut info = String::new();
                for (variant, variants) in r.iter().enumerate() {
                    self.get_variant_name(endpoints, endpoint_index, variant)
                        .map(|x| info.push_str(&format!(" | {x}")));
                    for (beam, entry) in variants.iter().enumerate() {
                        if let Ok(entry) = entry {
                            let beam = if beam > 0 {
                                format!(" ~ #{beam}")
                            } else {
                                String::new()
                            };
                            let duration_info = format!(" {:.1}s", entry.duration);
                            let tokens_info = entry
                                .total_tokens
                                .map(|tokens| format!(" {} t", tokens))
                                .unwrap_or_default();
                            let tokens_per_sec_info = entry
                                .total_tokens
                                .map(|tokens| {
                                    if entry.duration > 0.0 {
                                        format!(" {:.0} t/s", tokens as f64 / entry.duration)
                                    } else {
                                        String::new()
                                    }
                                })
                                .unwrap_or_default();
                            let logprob_info = entry
                                .logprob
                                .map(|logprob| format!(" {:.1}%", prob::logprob_to_prob(logprob)))
                                .unwrap_or_default();
                            info.push_str(&format!(
                                "{}{}{}{}{}",
                                beam, duration_info, tokens_info, tokens_per_sec_info, logprob_info,
                            ));
                        }
                    }
                }
                info
            })
            .unwrap_or_default()
    }

    fn get_model_name_multi(
        &self,
        endpoints: &[EndpointConfig],
        endpoint: usize,
        variant: usize,
        beam: usize,
        multi: usize,
    ) -> String {
        let variant_name = self.get_variant_name(endpoints, endpoint, variant);
        let mut name = endpoints[endpoint].name.to_string();
        let mut open = false;
        if let Some(variant_name) = *variant_name {
            open = true;
            name.push_str(" (");
            name.push_str(&variant_name);
        }
        if beam > 0 {
            if !open {
                open = true;
                name.push_str(" (");
            }
            name.push_str(&format!("#{}", beam));
        }
        if multi > 0 {
            if !open {
                open = true;
                name.push_str(" (");
            }
            name.push_str(&format!("${}", multi));
        }
        if open {
            name.push(')');
        }
        name
    }

    fn get_model_name(
        &self,
        endpoints: &[EndpointConfig],
        endpoint: usize,
        variant: usize,
    ) -> String {
        self.get_model_name_multi(endpoints, endpoint, variant, 0, 0)
    }

    fn get_variant_name(
        &self,
        endpoints: &[EndpointConfig],
        endpoint: usize,
        variant: usize,
    ) -> Box<Option<String>> {
        let endpoint = &endpoints[endpoint];
        match &endpoint.config {
            EndpointTypeConfig::OpenAI { variants, .. }
            | EndpointTypeConfig::Anthropic { variants, .. } => {
                if let Some(variants) = variants {
                    if let Some(variant) = variants.get(variant) {
                        return variant.name.clone();
                    } else {
                        assert!(variant == 0);
                    }
                }
                Box::new(None)
            }
            EndpointTypeConfig::Patchpal { .. } => Box::new(None),
        }
    }

    fn process_results(
        &self,
        resolved_conflicts: &mut Vec<ResolvedConflict>,
        resolver_errors: &mut ResolverErrors,
        results: &Vec<(Result<ApiResponse>, usize)>,
        conflict: &Conflict,
        endpoints: &[EndpointConfig],
    ) {
        // Validate that the content starts with head_context and ends with tail_context
        for result in results {
            let endpoint = result.1;
            let result = match &result.0 {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("Skipping {} due to error: {}", endpoints[endpoint].name, e);
                    continue;
                }
            };

            for (variant, api_response_variant) in result.iter().enumerate() {
                for (beam, api_response_entry) in api_response_variant.iter().enumerate() {
                    let api_response_entry = match api_response_entry {
                        Ok(api_response_entry) => api_response_entry,
                        Err(e) => {
                            let model = self.get_model_name(endpoints, endpoint, variant);
                            log::warn!("Skipping {} - {}", model, e);
                            *resolver_errors.errors.entry(model).or_insert(0) += 1;
                            continue;
                        }
                    };

                    let resolved_strings = match self.parse_response(&api_response_entry.response) {
                        Ok(resolved_strings) => resolved_strings,
                        Err(e) => {
                            let model = self.get_model_name(endpoints, endpoint, variant);
                            log::warn!("Skipping {} - {}", model, e);
                            *resolver_errors.errors.entry(model).or_insert(0) += 1;
                            continue;
                        }
                    };
                    assert!(!resolved_strings.is_empty());
                    assert!(!api_response_entry.response.is_empty());

                    let mut seen_resolved = std::collections::HashMap::new();
                    for (multi, resolved_string) in resolved_strings.iter().enumerate() {
                        let model =
                            self.get_model_name_multi(endpoints, endpoint, variant, beam, multi);
                        if !resolved_string.starts_with(&conflict.head_context) {
                            log::warn!("Skipping {} - doesn't start with head context", model);
                            let len = conflict.head_context.len().min(resolved_string.len());
                            let diff = ConflictResolver::create_diff(
                                &conflict.head_context,
                                &resolved_string[..len],
                                1,
                            );
                            log::trace!("HeadContextDiff:\n{}", diff);
                            *resolver_errors.errors.entry(model).or_insert(0) += 1;

                            continue;
                        }
                        let leading_tail_context = &format!("\n{}", &conflict.tail_context);
                        if !resolved_string.ends_with(leading_tail_context) {
                            log::warn!("Skipping {} - doesn't end with tail context", model);
                            let diff = ConflictResolver::create_diff(
                                &resolved_string[resolved_string
                                    .len()
                                    .saturating_sub(leading_tail_context.len())
                                    .max(0)..],
                                leading_tail_context,
                                1,
                            );
                            log::trace!("TailContextDiff:\n{}", diff);
                            *resolver_errors.errors.entry(model).or_insert(0) += 1;
                            continue;
                        }
                        //reduce resolved to the range between head_context and tail_context
                        let resolved_content = resolved_string[conflict.head_context.len()
                            ..resolved_string.len() - conflict.tail_context.len()]
                            .to_string();
                        if !resolved_content.is_empty() && !resolved_content.ends_with('\n') {
                            log::error!(
                                "Skipping {} - resolved content is not newline terminated",
                                model
                            );
                            log::trace!("ResolvedContent:\n{}", resolved_content);
                            *resolver_errors.errors.entry(model).or_insert(0) += 1;
                            continue;
                        }
                        // Check if this resolved_content is already in the results
                        let key = (endpoint, resolved_content.clone());
                        if seen_resolved.contains_key(&key) {
                            log::debug!("Skipping {} - duplicate resolved conflict", model);
                            continue;
                        }
                        seen_resolved.insert(key, model.clone());

                        let total_tokens = api_response_entry.total_tokens;
                        let logprob = api_response_entry.logprob;
                        let duration = api_response_entry.duration;
                        resolved_conflicts.push(ResolvedConflict {
                            conflict: conflict.clone(),
                            resolved_version: resolved_content,
                            model,
                            duration,
                            total_tokens,
                            logprob,
                        });
                    }
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
{s}{diff_end}"#,
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

Rewrite the {nr_head_context_lines} line{head_plural} after {code_start} and the {nr_tail_context_lines} line{tail_plural} before {code_end} exactly the same, including all empty lines."#,
            patch_start = Self::PATCH_START,
            patch_end = Self::PATCH_END,
            code_start = Self::CODE_START,
            code_end = Self::CODE_END,
            patched_code_start = Self::PATCHED_CODE_START,
            patched_code_end = Self::PATCHED_CODE_END,
            nr_head_context_lines = conflict.nr_head_context_lines,
            nr_tail_context_lines = conflict.nr_tail_context_lines,
            head_plural = if conflict.nr_head_context_lines != 1 {
                "s"
            } else {
                ""
            },
            tail_plural = if conflict.nr_tail_context_lines != 1 {
                "s"
            } else {
                ""
            }
        )
    }

    fn create_training() -> String {
        format!(
            r#"Learn from the following training example:

{patch_start}
@@ -1,7 +1,7 @@
 
 extern const struct feature default_feat;
 
-static inline const struct feature *get_extra_something(struct object *obj)
+static inline const struct feature *get_special_something(struct device *dev)
 {{
 	return &default_feat;
 }}
{patch_end}

{code_start}

extern struct feat feat;

static inline struct feat *get_extra_something(double option, struct device *obj, int param)
 {{	
	return &feat;
}}
{code_end}

{patched_code_start}

extern struct feat feat;

static inline struct feat *get_special_something(double option, struct device *dev, int param)
 {{	
	return &feat;
}}
{patched_code_end}"#,
            patch_start = ConflictResolver::PATCH_START,
            patch_end = ConflictResolver::PATCH_END,
            code_start = ConflictResolver::CODE_START,
            code_end = ConflictResolver::CODE_END,
            patched_code_start = ConflictResolver::PATCHED_CODE_START,
            patched_code_end = ConflictResolver::PATCHED_CODE_END,
        )
    }

    fn create_message(&self, patch: &String, code: &String) -> String {
        format!(
            r#"{patch_start}
{patch}{patch_end}

{code_start}
{code}{code_end}"#,
            patch_start = Self::PATCH_START,
            patch_end = Self::PATCH_END,
            code_start = Self::CODE_START,
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

        Self::create_diff(&base, &remote, self.context_lines.patch_context_lines)
    }

    pub fn create_diff(base: &str, remote: &str, patch_context_lines: u32) -> String {
        use imara_diff::{Algorithm, BasicLineDiffPrinter, Diff, InternedInput, UnifiedDiffConfig};
        let input = InternedInput::new(base, remote);
        let mut diff = Diff::compute(Algorithm::Histogram, &input);
        diff.postprocess_lines(&input);
        let mut config = UnifiedDiffConfig::default();
        config.context_len(patch_context_lines);
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
    fn parse_response(&self, response: &String) -> Result<Vec<String>> {
        let start_marker = format!("{}\n", Self::PATCHED_CODE_START);
        let end_marker = Self::PATCHED_CODE_END;

        log::info!("Response:\n{}", response);

        let mut results = Vec::new();
        let mut err: Option<Result<Vec<String>, anyhow::Error>> = None;
        let mut start = 0;

        while let Some(start_pos) = response[start..].find(&start_marker) {
            let start_pos = start + start_pos + start_marker.len();
            let end_pos = response[start_pos..].find(end_marker);
            if end_pos.is_none() {
                err = Some(Err(anyhow::anyhow!(
                    "Invalid format: missing {}",
                    Self::PATCHED_CODE_END
                )));
                break;
            }

            let end_pos = start_pos + end_pos.unwrap();

            let content = &response[start_pos..end_pos];
            results.push(content.to_string());

            start = end_pos + end_marker.len();
        }

        if results.is_empty() {
            match err {
                Some(err) => err,
                None => Err(anyhow::anyhow!("No code blocks found in response")),
            }
        } else {
            Ok(results)
        }
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
