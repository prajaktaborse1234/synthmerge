// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::config::{Config, EndpointTypeConfig};
use crate::conflict_resolver::{Conflict, ConflictResolver};
use crate::git_utils::GitUtils;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

#[derive(Debug)]
pub struct TestEntry {
    patch: String,
    code: String,
    patch_commit_hash: String,
    code_commit_hash: String,
    patched_code: String,
    filename: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TestResult {
    entry_index: usize,
    model: String,
    correct: bool,
    correct_aligned: bool,
    correct_stripped: bool,
    duration: f64,
    tokens: Option<u64>,
    failed_patched_code: Option<String>,
    error: bool,
    patch_commit_hash: String,
    code_commit_hash: String,
}

#[derive(Debug)]
struct ModelStats {
    total: usize,
    correct: usize,
    correct_aligned: usize,
    correct_stripped: usize,
    error: usize,
    accuracy: f64,
    accuracy_aligned: f64,
    accuracy_stripped: f64,
    error_rate: f64,
    avg_tokens: f64,
    avg_duration: f64,
}

#[derive(Debug)]
pub struct Bench {
    results: Vec<TestResult>,
    model_stats: HashMap<String, ModelStats>,
    git_diffs: HashMap<String, String>,
}

impl Default for Bench {
    fn default() -> Self {
        Self::new()
    }
}

impl Bench {
    pub fn new() -> Self {
        Bench {
            results: Vec::new(),
            model_stats: HashMap::new(),
            git_diffs: HashMap::new(),
        }
    }

    pub fn load_database<P: AsRef<Path>>(path: P) -> Result<Vec<TestEntry>> {
        let file = File::open(path.as_ref())?;
        let mut reader = csv::Reader::from_reader(file);
        let mut entries = Vec::new();

        for result in reader.records() {
            let record = result?;
            if record.len() < 6 {
                continue;
            }
            let description = record
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("Failed to get description from CSV record"))?;
            let mut split_desc = description.splitn(2, " / ");
            let code_commit_hash = split_desc.next().unwrap_or("").trim().to_string();
            let code_commit_hash = format!("{}^", code_commit_hash);
            let mut split_desc = split_desc.next().unwrap_or("").split('\n');
            let patch_commit_hash = split_desc.next().unwrap_or("").trim().to_string();
            let patch = record.get(3).unwrap_or("").to_string();
            let code = record.get(4).unwrap_or("").to_string();
            let patched_code = record.get(5).unwrap_or("").to_string();
            let filename = split_desc.next().unwrap_or("").trim().to_string();

            let entry = TestEntry {
                patch,
                code,
                patch_commit_hash,
                code_commit_hash,
                patched_code,
                filename,
            };
            entries.push(entry);
        }

        Ok(entries)
    }

    fn save_checkpoint<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let file = File::create(path.as_ref())?;
        let mut writer = csv::Writer::from_writer(file);
        for result in &self.results {
            writer.serialize(result)?;
        }
        writer.flush()?;
        self.calculate_stats();
        Ok(())
    }

    fn load_checkpoint<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if !path.as_ref().exists() {
            return Ok(());
        }

        let file = File::open(path.as_ref())?;
        let mut reader = csv::Reader::from_reader(file);

        for result in reader.deserialize() {
            let result: TestResult = result.with_context(|| "Failed to parse test result")?;
            self.results.push(result);
        }

        self.calculate_stats();

        Ok(())
    }

    fn calculate_stats(&mut self) {
        // Initialize stats for all models
        let mut model_totals = HashMap::new();
        let mut model_correct = HashMap::new();
        let mut model_correct_aligned = HashMap::new();
        let mut model_correct_stripped = HashMap::new();
        let mut model_tokens = HashMap::new();
        let mut model_durations = HashMap::new();
        let mut model_errors = HashMap::new();

        // Collect all results by model
        for result in &self.results {
            let model = &result.model;
            *model_totals.entry(model.clone()).or_insert(0) += 1;
            if result.correct {
                *model_correct.entry(model.clone()).or_insert(0) += 1;
            }
            if result.correct_aligned {
                *model_correct_aligned.entry(model.clone()).or_insert(0) += 1;
            }
            if result.correct_stripped {
                *model_correct_stripped.entry(model.clone()).or_insert(0) += 1;
            }
            if let Some(tokens) = result.tokens {
                model_tokens
                    .entry(model.clone())
                    .or_insert_with(Vec::new)
                    .push(tokens);
            }
            model_durations
                .entry(model.clone())
                .or_insert_with(Vec::new)
                .push(result.duration);
            if result.error {
                *model_errors.entry(model.clone()).or_insert(0) += 1;
            }
        }

        // Calculate final stats
        self.model_stats.clear();
        for (model, total) in model_totals {
            let correct = model_correct.get(&model).copied().unwrap_or(0);
            let accuracy = if total > 0 {
                correct as f64 / total as f64
            } else {
                0.0
            };
            let correct_aligned = model_correct_aligned.get(&model).copied().unwrap_or(0);
            let accuracy_aligned = if total > 0 {
                correct_aligned as f64 / total as f64
            } else {
                0.0
            };
            let correct_stripped = model_correct_stripped.get(&model).copied().unwrap_or(0);
            let accuracy_stripped = if total > 0 {
                correct_stripped as f64 / total as f64
            } else {
                0.0
            };
            let error = model_errors.get(&model).copied().unwrap_or(0);
            let error_rate = if total > 0 {
                error as f64 / total as f64
            } else {
                0.0
            };

            let avg_tokens = model_tokens
                .get(&model)
                .map(|tokens| tokens.iter().sum::<u64>() as f64 / tokens.len() as f64)
                .unwrap_or(0.0);

            let avg_duration = model_durations
                .get(&model)
                .map(|durations| durations.iter().sum::<f64>() / durations.len() as f64)
                .unwrap_or(0.0);

            self.model_stats.insert(
                model,
                ModelStats {
                    total,
                    correct,
                    correct_aligned,
                    correct_stripped,
                    error,
                    accuracy,
                    accuracy_aligned,
                    accuracy_stripped,
                    error_rate,
                    avg_tokens,
                    avg_duration,
                },
            );
        }
        self.print_results();
    }

    fn print_results(&self) {
        println!("\n=== MODEL ACCURACY RESULTS ===");
        if self.model_stats.is_empty() {
            println!("No results available");
            return;
        }

        let mut sorted_stats: Vec<_> = self.model_stats.iter().collect();
        sorted_stats.sort_by(|a, b| b.1.accuracy.partial_cmp(&a.1.accuracy).unwrap());

        for (model, stats) in sorted_stats {
            println!("\nModel: {}", model);
            println!(
                "  Accuracy: {:.2}% ({}/{})",
                stats.accuracy * 100.0,
                stats.correct,
                stats.total
            );
            println!(
                "  Accuracy (aligned): {:.2}% ({}/{})",
                stats.accuracy_aligned * 100.0,
                stats.correct_aligned,
                stats.total
            );
            println!(
                "  Accuracy (stripped): {:.2}% ({}/{})",
                stats.accuracy_stripped * 100.0,
                stats.correct_stripped,
                stats.total
            );
            println!(
                "  Error Rate: {:.2}% ({}/{})",
                stats.error_rate * 100.0,
                stats.error,
                stats.total
            );
            println!("  Average tokens: {:.2}", stats.avg_tokens);
            println!("  Average duration: {:.2} s", stats.avg_duration);
        }
    }

    pub async fn run_test(
        &mut self,
        config: &Config,
        entries: &[TestEntry],
        checkpoint_interval: usize,
        checkpoint_path: Option<&str>,
        git_directories: Option<Vec<String>>,
        context_lines: u32,
    ) -> Result<()> {
        println!("Running statistics test on {} entries", entries.len());

        // Create a new GitUtils instance to find the commit hash
        let git_utils = GitUtils::new(context_lines);

        // Load existing checkpoint
        if let Some(path) = checkpoint_path {
            self.load_checkpoint(path)?;
            println!(
                "Loaded {} existing results from checkpoint",
                self.results.len()
            );
        }

        let mut modified = false;
        for (i, entry) in entries.iter().enumerate() {
            // Skip entries that are already processed
            if self.results.iter().any(|r| r.entry_index == i) {
                continue;
            }

            let processing_msg = format!("Processing entry {} of {}...", i + 1, entries.len());
            log::info!("{}", processing_msg);
            println!("{}", processing_msg);

            // Create conflict from test entry
            let conflict = self.create_conflict_from_entry(entry)?;

            let git_diff = self.git_diffs.get(&entry.patch_commit_hash).cloned();
            let git_diff = git_diff.or_else(|| {
                // cache only the current commit
                self.git_diffs.clear();

                // Find the commit hash from the patch_commit_hash
                let commit_hash = &entry.patch_commit_hash;
                // Extract the diff from git
                if let Some(dirs) = &git_directories {
                    // Try each directory
                    for dir in dirs {
                        if let Some(diff) = git_utils
                            .extract_diff_in_dir(commit_hash, Some(dir))
                            .unwrap_or(None)
                        {
                            // Store the diff for future use
                            self.git_diffs.insert(commit_hash.clone(), diff.clone());
                            return Some(diff);
                        }
                    }
                    None
                } else {
                    None
                }
            });

            let resolver = ConflictResolver::new(config, git_diff);

            let resolved_conflicts = resolver.resolve_conflicts(&[conflict]).await;
            match resolved_conflicts {
                Ok((resolved_conflicts, resolved_errors)) => {
                    if resolved_conflicts.is_empty() {
                        self.add_error_results_for_all_endpoints(config, i, entry);
                    } else {
                        for (model_name, error_count) in &resolved_errors.errors {
                            let test_result = TestResult {
                                entry_index: i,
                                model: model_name.clone(),
                                correct: false,
                                correct_aligned: false,
                                correct_stripped: false,
                                duration: 0.0,
                                tokens: None,
                                failed_patched_code: None,
                                error: true,
                                patch_commit_hash: entry.patch_commit_hash.clone(),
                                code_commit_hash: entry.code_commit_hash.clone(),
                            };
                            for _ in 0..*error_count {
                                self.results.push(test_result.clone());
                            }
                        }
                        for resolved_conflict in resolved_conflicts {
                            let test_result = TestResult {
                                entry_index: i,
                                model: resolved_conflict.model,
                                correct: resolved_conflict.resolved_version == entry.patched_code,
                                correct_aligned: self.aligned(
                                    &resolved_conflict.resolved_version,
                                    &entry.patched_code,
                                ),
                                correct_stripped: self.stripped(
                                    &resolved_conflict.resolved_version,
                                    &entry.patched_code,
                                ),
                                duration: resolved_conflict.duration,
                                tokens: resolved_conflict.total_tokens,
                                failed_patched_code: if resolved_conflict.resolved_version
                                    == entry.patched_code
                                {
                                    None
                                } else {
                                    Some(resolved_conflict.resolved_version)
                                },
                                error: false,
                                patch_commit_hash: entry.patch_commit_hash.clone(),
                                code_commit_hash: entry.code_commit_hash.clone(),
                            };
                            self.results.push(test_result);
                        }
                    }
                }
                Err(_e) => self.add_error_results_for_all_endpoints(config, i, entry),
            };

            modified = true;
            // Save checkpoint periodically
            if let Some(path) = checkpoint_path
                && (i + 1) % checkpoint_interval == 0
            {
                println!("Saving checkpoint...");
                self.save_checkpoint(path)?;
            }
        }

        // Save final checkpoint
        if modified && let Some(path) = checkpoint_path {
            println!("Saving final checkpoint...");
            self.save_checkpoint(path)?;
        }

        Ok(())
    }

    fn add_error_results_for_all_endpoints(
        &mut self,
        config: &Config,
        entry_index: usize,
        entry: &TestEntry,
    ) {
        let mut model_names = Vec::new();
        // Collect all model names from endpoints configuration
        for endpoint in config.get_all_endpoints() {
            match &endpoint.config {
                EndpointTypeConfig::OpenAI { params, .. } => {
                    if let Some(params) = params {
                        for param in params.iter() {
                            let variant = if let Some(variant) = &*param.variant {
                                format!("{} ({})", endpoint.name, variant)
                            } else {
                                endpoint.name.clone()
                            };
                            model_names.push(variant);
                        }
                    } else {
                        // No params, just the endpoint name
                        model_names.push(endpoint.name.clone());
                    }
                }
                EndpointTypeConfig::Patchpal { .. } => {
                    // For patchpal, we have 3 variants (as per existing logic)
                    for y in 0..3 {
                        model_names.push(format!("{} #{}", endpoint.name, y));
                    }
                }
            }
        }

        assert!(!model_names.is_empty());

        for model_name in model_names {
            let test_result = TestResult {
                entry_index,
                model: model_name,
                correct: false,
                correct_aligned: false,
                correct_stripped: false,
                duration: 0.0,
                tokens: None,
                failed_patched_code: None,
                error: true,
                patch_commit_hash: entry.patch_commit_hash.clone(),
                code_commit_hash: entry.code_commit_hash.clone(),
            };
            self.results.push(test_result);
        }
    }

    fn stripped(&self, resolved: &str, expected: &str) -> bool {
        self.__stripped(resolved) == self.__stripped(expected)
    }

    fn __stripped(&self, s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn aligned(&self, resolved: &str, expected: &str) -> bool {
        self.__aligned(resolved) == self.__aligned(expected)
    }

    fn __aligned(&self, s: &str) -> String {
        s.lines()
            .filter(|line| line.chars().any(|c| !c.is_whitespace()))
            .map(|line| {
                let mut result = String::new();
                let mut seen_non_whitespace = false;
                let mut last_was_whitespace = false;
                for c in line.chars() {
                    if c.is_whitespace() {
                        if !seen_non_whitespace {
                            result.push(c);
                        } else {
                            last_was_whitespace = true;
                        }
                    } else {
                        if last_was_whitespace {
                            result.push(' ');
                        }
                        result.push(c);
                        seen_non_whitespace = true;
                    }
                }
                result
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn create_conflict_from_entry(&self, entry: &TestEntry) -> Result<Conflict> {
        // For simplicity, we'll create a basic conflict structure
        // In a real test, this would be more sophisticated
        let mut base_lines = Vec::new();
        let mut remote_lines = Vec::new();
        let mut nr_head_context_lines = 0;
        let mut found_first_change = false;
        let mut line_count = 0;
        let line_number_re = regex::Regex::new(r"^@@ -\d+,\d+ \+\d+,\d+ @@").unwrap();
        for line in entry.patch.split_inclusive('\n') {
            if line_number_re.is_match(line) {
                continue;
            }
            if let Some(line) = line.strip_prefix('+') {
                remote_lines.push(line.to_string());
                if !found_first_change {
                    nr_head_context_lines = line_count;
                    found_first_change = true;
                }
                line_count = 0;
                continue;
            } else if let Some(line) = line.strip_prefix('-') {
                base_lines.push(line.to_string());
                if !found_first_change {
                    nr_head_context_lines = line_count;
                    found_first_change = true;
                }
                line_count = 0;
                continue;
            }

            base_lines.push(line.to_string());
            remote_lines.push(line.to_string());
            line_count += 1;
        }
        let nr_tail_context_lines = line_count;
        let base = base_lines.join("");
        let remote = remote_lines.join("");
        Ok(Conflict {
            file_path: entry.filename.clone(),
            local: entry.code.clone(),
            base,
            remote,
            head_context: String::new(),
            tail_context: String::new(),
            start_line: 1,
            remote_start: 1,
            nr_head_context_lines,
            nr_tail_context_lines,
            marker_size: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_result_serialization() {
        let result = TestResult {
            entry_index: 0,
            model: "test_model".to_string(),
            correct: true,
            correct_aligned: true,
            correct_stripped: true,
            duration: 0.0,
            tokens: None,
            failed_patched_code: None,
            error: false,
            patch_commit_hash: "abc123".to_string(),
            code_commit_hash: "def456".to_string(),
        };

        let serialized = serde_json::to_string(&result).unwrap();
        let deserialized: TestResult = serde_json::from_str(&serialized).unwrap();

        assert_eq!(result.entry_index, deserialized.entry_index);
        assert_eq!(result.model, deserialized.model);
        assert_eq!(result.correct, deserialized.correct);
        assert_eq!(result.correct_aligned, deserialized.correct_aligned);
        assert_eq!(result.correct_stripped, deserialized.correct_stripped);
        assert_eq!(result.duration, deserialized.duration);
        assert_eq!(result.tokens, deserialized.tokens);
        assert_eq!(result.failed_patched_code, deserialized.failed_patched_code);
        assert_eq!(result.error, deserialized.error);
        assert_eq!(result.patch_commit_hash, deserialized.patch_commit_hash);
        assert_eq!(result.code_commit_hash, deserialized.code_commit_hash);
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
