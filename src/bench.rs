// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::config::Config;
use crate::conflict_resolver::{Conflict, ConflictResolver};
use crate::git_utils::GitUtils;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct TestEntry {
    pub patch: String,
    pub code: String,
    pub patch_commit_hash: String,
    pub code_commit_hash: String,
    pub patched_code: String,
    pub filename: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TestResult {
    pub entry_index: usize,
    pub model: String,
    pub correct: bool,
    pub correct_aligned: bool,
    pub correct_stripped: bool,
    pub duration: f64,
    pub tokens: Option<u64>,
    pub failed_patched_code: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStats {
    pub total: usize,
    pub correct: usize,
    pub correct_aligned: usize,
    pub correct_stripped: usize,
    pub error: usize,
    pub accuracy: f64,
    pub accuracy_aligned: f64,
    pub accuracy_stripped: f64,
    pub error_rate: f64,
    pub avg_tokens: f64,
    pub avg_duration: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bench {
    pub results: Vec<TestResult>,
    pub model_stats: HashMap<String, ModelStats>,
    pub git_diffs: HashMap<String, String>,
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

    pub fn save_checkpoint<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path.as_ref())?;
        let mut writer = csv::Writer::from_writer(file);
        for result in &self.results {
            writer.serialize(result)?;
        }
        writer.flush()?;
        Ok(())
    }

    pub fn load_checkpoint<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if !path.as_ref().exists() {
            return Ok(());
        }

        let file = File::open(path.as_ref())?;
        let mut reader = csv::Reader::from_reader(file);

        for result in reader.deserialize() {
            let result: TestResult = result.with_context(|| "Failed to parse test result")?;
            self.results.push(result);
        }

        Ok(())
    }

    pub fn calculate_stats(&mut self) {
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
            if result.error.is_some() {
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

        for (i, entry) in entries.iter().enumerate() {
            // Skip entries that are already processed
            if self.results.iter().any(|r| r.entry_index == i) {
                continue;
            }

            println!("Processing entry {} of {}...", i + 1, entries.len());

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
            let resolved_conflicts = match resolved_conflicts {
                Ok(conflicts) => {
                    if conflicts.is_empty() {
                        let test_result = TestResult {
                            entry_index: i,
                            model: "error".to_string(),
                            correct: false,
                            correct_aligned: false,
                            correct_stripped: false,
                            duration: 0.0,
                            tokens: None,
                            failed_patched_code: None,
                            error: Some("No conflicts resolved".to_string()),
                        };
                        self.results.push(test_result);
                        continue;
                    }
                    conflicts
                }
                Err(e) => {
                    let test_result = TestResult {
                        entry_index: i,
                        model: "error".to_string(),
                        correct: false,
                        correct_aligned: false,
                        correct_stripped: false,
                        duration: 0.0,
                        tokens: None,
                        failed_patched_code: None,
                        error: Some(e.to_string()),
                    };
                    self.results.push(test_result);
                    continue;
                }
            };
            for resolved_conflict in resolved_conflicts {
                let test_result = TestResult {
                    entry_index: i,
                    model: resolved_conflict.dedup.model,
                    correct: resolved_conflict.dedup.resolved_version == entry.patched_code,
                    correct_aligned: self.aligned(
                        &resolved_conflict.dedup.resolved_version,
                        &entry.patched_code,
                    ),
                    correct_stripped: self.stripped(
                        &resolved_conflict.dedup.resolved_version,
                        &entry.patched_code,
                    ),
                    duration: resolved_conflict.duration,
                    tokens: resolved_conflict.total_tokens,
                    failed_patched_code: if resolved_conflict.dedup.resolved_version
                        == entry.patched_code
                    {
                        None
                    } else {
                        Some(resolved_conflict.dedup.resolved_version)
                    },
                    error: None,
                };
                self.results.push(test_result);
            }

            // Save checkpoint periodically
            if let Some(path) = checkpoint_path
                && (i + 1) % checkpoint_interval == 0
            {
                println!("Saving checkpoint...");
                self.save_checkpoint(path)?;
                self.calculate_stats();
            }
        }

        // Final calculation
        self.calculate_stats();
        Ok(())
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
        for line in entry.patch.split_inclusive('\n') {
            if let Some(line) = line.strip_prefix('+') {
                remote_lines.push(line.to_string());
                if !found_first_change {
                    nr_head_context_lines = line_count - 1;
                    found_first_change = true;
                }
                line_count = 0;
            } else if let Some(line) = line.strip_prefix('-') {
                base_lines.push(line.to_string());
                if !found_first_change {
                    nr_head_context_lines = line_count - 1;
                    found_first_change = true;
                }
                line_count = 0;
            } else {
                base_lines.push(line.to_string());
                remote_lines.push(line.to_string());
            }
            line_count += 1;
        }
        let nr_tail_context_lines = line_count - 1;
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_entry_serialization() {
        let entry = TestEntry {
            patch: "patch content".to_string(),
            code: "code content".to_string(),
            patch_commit_hash: "abc123".to_string(),
            code_commit_hash: "def456".to_string(),
            patched_code: "patched code content".to_string(),
            filename: "test_file.txt".to_string(),
        };

        let serialized = serde_json::to_string(&entry).unwrap();
        let deserialized: TestEntry = serde_json::from_str(&serialized).unwrap();

        assert_eq!(entry.patch, deserialized.patch);
        assert_eq!(entry.code, deserialized.code);
        assert_eq!(entry.patch_commit_hash, deserialized.patch_commit_hash);
        assert_eq!(entry.code_commit_hash, deserialized.code_commit_hash);
        assert_eq!(entry.patched_code, deserialized.patched_code);
        assert_eq!(entry.filename, deserialized.filename);
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
