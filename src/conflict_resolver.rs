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
}

impl ConflictResolver {
    pub fn new(config: Config, verbose: bool) -> Self {
        ConflictResolver { config, verbose }
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
            let prompt = self.create_prompt();
            let patch = self.create_patch(conflict);
            let code = self.create_code(conflict);
            let message = self.create_message(conflict);
            if self.verbose {
                println!("Message:\n{}", message);
            }

	    // Try to resolve with all endpoints in parallel
            let mut futures = Vec::new();
            for (order, endpoint) in endpoints.iter().enumerate() {
                let prompt = prompt.clone();
                let message = message.clone();
                let patch = patch.clone();
                let code = code.clone();
                let client = ApiClient::new(endpoint.clone(), self.verbose);
                let name = endpoint.name.clone();
                let handle = tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    let result = client.query(&prompt, &message, &patch, &code).await;
                    let duration = start.elapsed().as_secs();
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
                        println!(" - {} completed in {} seconds", name, duration);
                        results.push((result, order));
		    }
		    Err(e) => return Err(anyhow::anyhow!("Task failed: {}", e)),
                }
	    }

	    results.sort_by_key(|k| k.1);
            let results: Vec<_> = results.into_iter().map(|r| r.0).collect();

            if self.verbose {
                println!("resolved:\n{:?}", results);
            }

            // Validate that the content starts with head_context and ends with tail_context
            for (i, result) in results.iter().enumerate() {
                let result = match result {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("Warning: Skipping {} due to error: {}", endpoints[i].name, e);
                        continue;
                    }
                };

                let resolved = self.parse_response(result);
                let resolved = match resolved {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("Warning: Skipping {} due to error: {}", endpoints[i].name, e);
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
                        eprintln!("Warning: Skipped {} - doesn't start with head context", model);
                        continue;
                    }
                    if self.verbose {
                        println!("tail_context: {:?}", conflict.tail_context);
                    }
                    if !resolved_string.ends_with(&conflict.tail_context) {
                        eprintln!("Warning: Skipped {} - doesn't end with tail context", model);
                        continue;
                    }
                    //reduce resolved to the range between head_context and tail_context
                    let resolved_content = resolved_string[conflict.head_context.len()
                        ..resolved_string.len() - conflict.tail_context.len()]
                        .to_string();

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
    fn create_prompt(&self) -> String {
        r#"Apply the patch between <|patch_start|><|patch_end|> to the code between <|code_start|><|code_end|>.

Reason about the patch and don't alter any line of code that doesn't start with a + or - sign in the patch.

Finally answer with the patched code between <|code_start|><|code_end|>.

Rewrite the 3 lines after <|code_start|> and the 3 lines before <|code_end|> exactly the same, including all empty lines."#.to_string()
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
    fn parse_response(&self, response: &str) -> Result<Vec<String>> {
        let start_marker = "<|code_start|>\n";
        let end_marker = "<|code_end|>";
        let mut results = Vec::new();
        let mut start = 0;

        if self.verbose {
            println!("Response:\n{}", response);
        }

        while let Some(start_pos) = response[start..].find(start_marker) {
            let start_pos = start + start_pos;
            let end_pos = response[start_pos..]
                .find(end_marker)
                .ok_or_else(|| anyhow::anyhow!("Invalid format: missing <|code_end|>"))?;

            let end_pos = start_pos + end_pos;
            if start_pos > end_pos {
                return Err(anyhow::anyhow!(
                    "Invalid format: <|code_start|> appears after <|code_end|>"
                ));
            }

            let content_start = start_pos + start_marker.len();
            let content_end = end_pos;

            let content = &response[content_start..content_end];
            results.push(content.to_string());

            start = end_pos + end_marker.len();
        }

        Ok(results)
    }
}
