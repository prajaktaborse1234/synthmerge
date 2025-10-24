// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::api_client::ApiClient;
use crate::config::Config;
use crate::git_utils::{Conflict, ResolvedConflict};
use anyhow::Result;
use futures::future::select_all;

pub struct ConflictResolver {
    config: Config,
    verbose: bool,
    git_diff: Option<String>,
}

impl ConflictResolver {
    pub fn new(config: Config, verbose: bool, git_diff: Option<String>) -> Self {
        ConflictResolver {
            config,
            verbose,
            git_diff: Self::__git_diff(git_diff),
        }
    }

    /// Resolve all conflicts using AI
    pub async fn resolve_conflicts(self, conflicts: &[Conflict]) -> Result<Vec<ResolvedConflict>> {
        let mut resolved_conflicts = Vec::new();
        let config = &self.config;

        for (i, conflict) in conflicts.iter().enumerate() {
            println!(
                "Resolving conflict {} of {} in {}:{}",
                i + 1,
                conflicts.len(),
                conflict.file_path,
                conflict.start_line
            );

            // Get all endpoints
            let endpoints = config.get_all_endpoints();

            // Create the prompt for AI resolution
            let prompt = self.create_prompt(conflict);
            let patch = self.create_patch(conflict);
            let code = self.create_code(conflict);
            let message = self.create_message(conflict);
            if self.verbose {
                println!("Message:\n{}", message);
            }

            // Try to resolve with all endpoints in parallel
            let mut futures = Vec::new();
            for (order, endpoint) in endpoints.iter().enumerate() {
                let client = ApiClient::new(endpoint.clone(), self.verbose);
                let name = endpoint.name.clone();
                let api_request = crate::api_client::ApiRequest {
                    prompt: prompt.clone(),
                    message: message.clone(),
                    patch: patch.clone(),
                    code: code.clone(),
                    config: endpoint.clone(),
                    git_diff: self.git_diff.clone(),
                };
                let handle = tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    let result = client.query(&api_request).await;
                    let duration = start.elapsed();
                    (result, duration, name, order)
                });
                futures.push(handle);
            }

            let mut results = Vec::new();
            while !futures.is_empty() {
                let (result, _, remaining) = select_all(futures).await;
                futures = remaining;
                match result {
                    Ok((result, duration, name, order)) => {
                        let duration = duration.as_secs_f64();
                        println!(
                            " - {} completed in {:.2} s{}",
                            name,
                            duration,
                            result
                                .as_ref()
                                .and_then(|r| Ok(r.total_tokens.map(|t| format!(
                                    " - tokens {} - {:.2} t/s",
                                    t.to_string(),
                                    t as f64 / duration
                                ))))
                                .unwrap_or_default()
                                .unwrap_or_default()
                        );
                        results.push((result, order));
                    }
                    Err(e) => return Err(anyhow::anyhow!("Task failed: {}", e)),
                }
            }

            results.sort_by_key(|k| k.1);
            let results: Vec<_> = results.into_iter().map(|r| r.0).collect();

            // Validate that the content starts with head_context and ends with tail_context
            for (i, result) in results.iter().enumerate() {
                let result = match result {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!(
                            "Warning: Skipping {} due to error: {}",
                            endpoints[i].name, e
                        );
                        continue;
                    }
                };

                let resolved = self.parse_response(result);
                let resolved = match resolved {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!(
                            "Warning: Skipping {} due to error: {}",
                            endpoints[i].name, e
                        );
                        continue;
                    }
                };

                for (n, resolved_string) in resolved.iter().enumerate() {
                    let model = if n > 0 {
                        format!("{} #{}", endpoints[i].name, n + 1)
                    } else {
                        endpoints[i].name.clone()
                    };
                    if !resolved_string.starts_with(&conflict.head_context) {
                        eprintln!(
                            "Warning: Skipped {} - doesn't start with head context",
                            model
                        );
                        continue;
                    }
                    if !resolved_string.ends_with(&conflict.tail_context) {
                        eprintln!("Warning: Skipped {} - doesn't end with tail context", model);
                        continue;
                    }
                    //reduce resolved to the range between head_context and tail_context
                    let resolved_content = resolved_string[conflict.head_context.len()
                        ..resolved_string.len() - conflict.tail_context.len()]
                        .to_string();
                    if !resolved_content.is_empty() && !resolved_content.ends_with('\n') {
                        eprintln!(
                            "Warning: Skipped {} - resolved content is not newline terminated",
                            model
                        );
                        continue;
                    }

                    resolved_conflicts.push(ResolvedConflict {
                        conflict: conflict.clone(),
                        resolved_version: resolved_content,
                        model,
                    });
                }
            }
        }

        Ok(resolved_conflicts)
    }

    /// Create a prompt for the AI to resolve the conflict
    fn __git_diff(git_diff: Option<String>) -> Option<String> {
        git_diff.map(|s| {
            format!(
                r#"The PATCH originates from the DIFF between <|diff_start|><|diff_end|>.

<|diff_start|>
{}
<|diff_end|>"#,
                s
            )
        })
    }

    /// Create a prompt for the AI to resolve the conflict
    fn create_prompt(&self, conflict: &Conflict) -> String {
        format!(
            r#"Apply the PATCH between <|patch_start|><|patch_end|> to the CODE between <|code_start|><|code_end|>.

Write the reasoning about the PATCH focusing only on the modifications done in the + or - lines of the PATCH and don't make other modifications to the CODE.

FINALLY write the final PATCHED CODE between <|patched_code_start|><|patched_code_end|> instead of markdown fences.

Rewrite the {} lines after <|code_start|> and the {} lines before <|code_end|> exactly the same, including all empty lines."#,
            conflict.nr_head_context_lines, conflict.nr_tail_context_lines
        )
    }

    fn create_message(&self, conflict: &Conflict) -> String {
        let patch = self.create_patch(conflict);
        let code = self.create_code(conflict);

        format!(
            r#"<|patch_start|>
{patch}<|patch_end|>

<|code_start|>
{code}<|code_end|>
"#
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
        diff.unified_diff(
            &BasicLineDiffPrinter(&input.interner),
            UnifiedDiffConfig::default(),
            &input,
        )
        .to_string()
    }

    fn create_code(&self, conflict: &Conflict) -> String {
        format!(
            "{}{}{}",
            conflict.head_context, conflict.local, conflict.tail_context
        )
    }

    /// Parse the API response into 3 solutions
    fn parse_response(&self, response: &crate::api_client::ApiResponse) -> Result<Vec<String>> {
        let start_marker = "<|patched_code_start|>\n";
        let end_marker = "<|patched_code_end|>";
        let mut results = Vec::new();
        let mut start = 0;

        if self.verbose {
            println!("Response:\n{}", response.response);
        }

        while let Some(start_pos) = response.response[start..].find(start_marker) {
            let start_pos = start + start_pos;
            let end_pos = response.response[start_pos..]
                .find(end_marker)
                .ok_or_else(|| anyhow::anyhow!("Invalid format: missing <|patched_code_end|>"))?;

            let end_pos = start_pos + end_pos;
            if start_pos > end_pos {
                return Err(anyhow::anyhow!(
                    "Invalid format: <|patched_code_start|> appears after <|patched_code_end|>"
                ));
            }

            let content_start = start_pos + start_marker.len();
            let content_end = end_pos;

            let content = &response.response[content_start..content_end];
            results.push(content.to_string());

            start = end_pos + end_marker.len();
        }

        if results.is_empty() {
            return Err(anyhow::anyhow!("No code blocks found in response"));
        }

        Ok(results)
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
