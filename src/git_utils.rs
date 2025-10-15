// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::process::Command;

pub struct GitUtils {
    verbose: bool,
}

impl GitUtils {
    pub fn new(verbose: bool) -> Self {
        GitUtils { verbose }
    }

    /// Check that git cherry-pick default is diff3 for merge.conflictStyle
    pub fn check_diff3(&self) -> Result<()> {
        // Check that git cherry-pick default is diff3 for merge.conflictStyle
        let output = Command::new("git")
            .args(["config", "--get", "merge.conflictStyle"])
            .output()
            .context("Failed to get git config")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to get merge.conflictStyle: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let config_value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if config_value != "diff3" {
            return Err(anyhow::anyhow!(
                "merge.conflictStyle is not set to 'diff3', it is set to '{}'",
                config_value
            ));
        }

        Ok(())
    }

    /// Find all conflict markers in the repository
    pub fn find_conflicts(&self) -> Result<Vec<Conflict>> {
        let mut conflicts = Vec::new();

        // Find all files that might contain conflicts
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .context("Failed to execute git status")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Git status failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let status_output = String::from_utf8_lossy(&output.stdout);
        for line in status_output.lines() {
            if line.starts_with("UU") {
                // Unmerged file
                let file_path = line[3..].trim();
                let conflict = self.parse_conflict_from_file(file_path)?;
                conflicts.extend(conflict);
            }
        }

        Ok(conflicts)
    }

    /// Parse conflicts from a specific file
    fn parse_conflict_from_file(&self, file_path: &str) -> Result<Vec<Conflict>> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path))?;

        let mut conflicts = Vec::new();
        let re = Regex::new(r"(?s)(<<<<<<< HEAD.*?=======.*?>>>>>>>.*?\n)").unwrap();

        for cap in re.captures_iter(&content) {
            let conflict_text = cap.get(0).unwrap().as_str();
            let start_line = content[..cap.get(0).unwrap().start()]
                .chars()
                .filter(|&c| c == '\n')
                .count()
                + 1;
            let conflict =
                self.parse_conflict_text(conflict_text, &content, start_line, file_path)?;
            conflicts.push(conflict);
        }

        Ok(conflicts)
    }

    /// Parse a conflict block into structured data
    fn parse_conflict_text(
        &self,
        conflict_text: &str,
        content: &str,
        start_line: usize,
        file_path: &str,
    ) -> Result<Conflict> {
        let conflict_lines: Vec<&str> = conflict_text.lines().collect();
        let nr_context_lines = 3;

        let local_start = conflict_lines
            .iter()
            .position(|&line| line == "<<<<<<< HEAD")
            .context("Failed to find HEAD marker")?;

        let base_start = conflict_lines
            .iter()
            .position(|&line| line.starts_with("|||||||"))
            .context("Failed to find base marker")?;

        let remote_start = conflict_lines
            .iter()
            .position(|&line| line == "=======")
            .context("Failed to find conflict marker")?;

        let remote_end = conflict_lines
            .iter()
            .position(|&line| line.starts_with(">>>>>>>"))
            .context("Failed to find conflict end marker")?;

        if remote_end <= remote_start || remote_start <= base_start || base_start <= local_start {
            return Err(anyhow::anyhow!("Invalid conflict markers"));
        }

        let local_lines: Vec<&str> = conflict_lines[local_start + 1..base_start].to_vec();
        let base_lines: Vec<&str> = conflict_lines[base_start + 1..remote_start].to_vec();
        let remote_lines: Vec<&str> = conflict_lines[remote_start + 1..remote_end].to_vec();

        let content_lines: Vec<&str> = content.lines().collect();

        let head_context_end = (start_line.saturating_sub(1)).max(0);
        let head_context_start = (head_context_end.saturating_sub(nr_context_lines)).max(0);
        let head_context_lines: Vec<&str> =
            content_lines[head_context_start..head_context_end].to_vec();

        let tail_context_start = (start_line + conflict_lines.len() - 1).min(content_lines.len());
        let tail_context_end = (tail_context_start + nr_context_lines).min(content_lines.len());
        let tail_context_lines: Vec<&str> =
            content_lines[tail_context_start..tail_context_end].to_vec();
        if self.verbose {
            println!("tail_context_lines: {:?}", tail_context_lines);
        }

        Ok(Conflict {
            file_path: file_path.to_string(),
            local: local_lines.join("\n") + "\n",
            base: base_lines.join("\n") + "\n",
            remote: remote_lines.join("\n") + "\n",
            head_context: head_context_lines.join("\n") + "\n",
            tail_context: tail_context_lines.join("\n") + "\n",
            start_line,
            remote_start,
        })
    }

    /// Apply resolved conflicts back to the repository
    pub fn apply_resolved_conflicts(&self, conflicts: &[ResolvedConflict]) -> Result<()> {
        let conflicts = Self::deduplicate_conflicts(conflicts);

        for conflict in conflicts.iter().rev() {
            println!(
                "Applying resolved conflict for: {} at line {}",
                conflict.conflict.file_path, conflict.conflict.start_line
            );

            // Read the file
            let mut content = fs::read_to_string(&conflict.conflict.file_path)
                .with_context(|| format!("Failed to read file: {}", conflict.conflict.file_path))?;

            // Split content into lines
            let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

            // Calculate the line where we want to insert the resolved content
            //print startline and remote start
            let insert_line = conflict.conflict.start_line + conflict.conflict.remote_start - 1;

            if self.verbose {
                println!("resolved_version: {:?}", conflict.resolved_version);
            }

            // Insert the resolved content with markers
            let marker_raw = "||||||| AI generated by";
            let marker = format!("{}: {}", marker_raw, conflict.model);
            let current_line = &lines[insert_line];
            if current_line != "=======" && !current_line.starts_with(marker_raw) {
                eprintln!(
                    "Error: Invalid conflict marker found at line {}",
                    insert_line + 1
                );
                continue;
            }
            lines.insert(insert_line, marker);
            let resolved_lines: Vec<String> = conflict
                .resolved_version
                .lines()
                .map(|s| s.to_string())
                .collect();
            for (i, line) in resolved_lines.iter().enumerate() {
                lines.insert(insert_line + 1 + i, line.clone());
            }

            content = lines.join("\n") + "\n";

            // Write back to file
            fs::write(&conflict.conflict.file_path, content).with_context(|| {
                format!("Failed to write file: {}", conflict.conflict.file_path)
            })?;
        }

        // Add Assisted-by line to merge message
        self.update_merge_message()?;

        Ok(())
    }

    fn deduplicate_conflicts(conflicts: &[ResolvedConflict]) -> Vec<ResolvedConflict> {
        use std::collections::HashMap;
        let mut map: HashMap<String, Vec<&ResolvedConflict>> = HashMap::new();

        // Group conflicts by resolved_version
        for conflict in conflicts {
            map.entry(conflict.resolved_version.clone())
                .or_default()
                .push(conflict);
        }

        // For each group, create a new conflict with combined model names
        let mut result = Vec::new();
        for (resolved_version, group) in map {
            let model = group
                .iter()
                .map(|c| c.model.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            // Use the first conflict in the group as the base
            let base_conflict = group[0];
            result.push(ResolvedConflict {
                conflict: base_conflict.conflict.clone(),
                resolved_version,
                model,
            });
        }

        // Restore original order
        let mut ordered_result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for original in conflicts {
            let key = &original.resolved_version;
            if seen.insert(key)
                && let Some(pos) = result.iter().position(|r| &r.resolved_version == key)
            {
                ordered_result.push(result[pos].clone());
            }
        }

        ordered_result
    }

    /// Update the git merge message to include Assisted-by line
    fn update_merge_message(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Failed to execute git rev-parse")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Git rev-parse failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let git_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let merge_msg_path = format!("{}/.git/MERGE_MSG", git_root);
        let merge_msg_content = match fs::read_to_string(&merge_msg_path) {
            Ok(content) => content,
            Err(_) => {
                eprintln!(
                    "If you use the AI generated code please add \"Assisted-by: synthmerge\""
                );
                return Ok(());
            }
        };

        let mut lines: Vec<String> = merge_msg_content.lines().map(|s| s.to_string()).collect();

        // Find the line before "# Conflicts:" or end of file
        let mut insert_pos = lines.len();
        for (i, line) in lines.iter().enumerate() {
            if line.trim() == "# Conflicts:" {
                insert_pos = i;
                break;
            }
        }

        // Go backwards to find the last non-empty line
        while insert_pos > 0 {
            insert_pos -= 1;
            if !lines[insert_pos].trim().is_empty() {
                break;
            }
        }

        // Insert the Assisted-by line after the last non-empty line
        let assisted_line = "Assisted-by: synthmerge".to_string();
        lines.insert(insert_pos + 1, assisted_line);

        let updated_content = lines.join("\n") + "\n";
        fs::write(&merge_msg_path, updated_content).with_context(|| {
            format!("Failed to write updated merge message: {}", merge_msg_path)
        })?;

        println!("Added \"Assisted-by: synthmerge\" to the .git/MERGE_MSG file");

        Ok(())
    }
}

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
}

#[derive(Debug, Clone)]
pub struct ResolvedConflict {
    pub conflict: Conflict,
    pub resolved_version: String,
    pub model: String,
}
