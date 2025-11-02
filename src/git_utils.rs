// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::conflict_resolver::{Conflict, DedupResolvedConflict, ResolvedConflict};
use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::process::Command;

pub struct GitUtils {
    context_lines: u32,
    in_rebase: bool,
}

impl GitUtils {
    const ASSISTED_BY_LINE: &str = concat!("Assisted-by: ", env!("CARGO_PKG_NAME"));
    const REBASE_MESSAGE_FILE: &str = "rebase-merge/message";
    const MERGE_MSG_FILE: &str = "MERGE_MSG";

    const HEAD_MARKER: &str = "<<<<<<< ";
    const BASE_MARKER: &str = "||||||| ";
    const CONFLICT_MARKER: &str = "=======";
    const END_MARKER: &str = ">>>>>>>";

    pub fn new(context_lines: u32) -> Self {
        GitUtils {
            context_lines,
            in_rebase: false,
        }
    }

    /// Check that git cherry-pick default is diff3 for merge.conflictStyle
    pub fn check_diff3(&self) -> Result<()> {
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
            .args(["diff", "--name-only", "--diff-filter=U"])
            .output()
            .context("Failed to execute git diff")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Git diff failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let diff_output = String::from_utf8_lossy(&output.stdout);
        for line in diff_output.lines() {
            let file_path = line.trim();
            let conflict = self.parse_conflict_from_file(file_path)?;
            conflicts.extend(conflict);
        }

        Ok(conflicts)
    }

    /// Parse conflicts from a specific file
    fn parse_conflict_from_file(&self, file_path: &str) -> Result<Vec<Conflict>> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path))?;

        let mut conflicts = Vec::new();
        let re = Regex::new(&format!(
            r"(?ms)(^{}HEAD.*?^{}.*?^{}.*?^{}.*?\n)",
            GitUtils::HEAD_MARKER,
            GitUtils::BASE_MARKER
                .chars()
                .map(|c| format!(r"\{}", c))
                .collect::<Vec<_>>()
                .join(""),
            GitUtils::CONFLICT_MARKER,
            GitUtils::END_MARKER,
        ))
        .unwrap();

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
        let conflict_lines: Vec<&str> = conflict_text.split_inclusive('\n').collect();

        let local_start = conflict_lines
            .iter()
            .position(|&line| line == format!("{}HEAD\n", GitUtils::HEAD_MARKER))
            .context("Failed to find HEAD marker")?;

        let base_start = conflict_lines
            .iter()
            .position(|&line| line.starts_with(GitUtils::BASE_MARKER))
            .context("Failed to find base marker")?;

        let remote_start = conflict_lines
            .iter()
            .position(|&line| line == format!("{}\n", GitUtils::CONFLICT_MARKER))
            .context("Failed to find conflict marker")?;

        let remote_end = conflict_lines
            .iter()
            .position(|&line| line.starts_with(GitUtils::END_MARKER))
            .context("Failed to find conflict end marker")?;

        if remote_end <= remote_start || remote_start <= base_start || base_start <= local_start {
            return Err(anyhow::anyhow!("Invalid conflict markers"));
        }

        let local_lines: Vec<&str> = conflict_lines[local_start + 1..base_start].to_vec();
        let base_lines: Vec<&str> = conflict_lines[base_start + 1..remote_start].to_vec();
        let remote_lines: Vec<&str> = conflict_lines[remote_start + 1..remote_end].to_vec();

        let content_lines: Vec<&str> = content.split_inclusive('\n').collect();

        let head_context_end = (start_line.saturating_sub(1)).max(0);
        let head_context_start =
            (head_context_end.saturating_sub(self.context_lines as usize)).max(0);
        let nr_head_context_lines = head_context_end - head_context_start;
        let head_content_lines = content_lines[..start_line].to_vec();
        let head_content_lines =
            Self::remove_conflict_markers(head_content_lines[..head_context_end].to_vec())?;
        let head_context_lines = head_content_lines[head_content_lines
            .len()
            .saturating_sub(self.context_lines as usize)
            .max(0)..]
            .to_vec();

        let tail_context_start = (start_line + conflict_lines.len() - 1).min(content_lines.len());
        let tail_context_end =
            (tail_context_start + self.context_lines as usize).min(content_lines.len());
        let nr_tail_context_lines = tail_context_end - tail_context_start;
        let tail_content_lines = content_lines[start_line + conflict_lines.len() - 1..].to_vec();
        let tail_content_lines = Self::remove_conflict_markers(tail_content_lines)?;
        let tail_context_lines = tail_content_lines
            [..tail_content_lines.len().min(self.context_lines as usize)]
            .to_vec();

        Ok(Conflict {
            file_path: file_path.to_string(),
            local: local_lines.join(""),
            base: base_lines.join(""),
            remote: remote_lines.join(""),
            head_context: head_context_lines.join(""),
            tail_context: tail_context_lines.join(""),
            start_line,
            remote_start,
            nr_head_context_lines,
            nr_tail_context_lines,
        })
    }

    /// Remove conflict markers from content
    fn remove_conflict_markers(content_lines: Vec<&str>) -> Result<Vec<&str>, anyhow::Error> {
        let mut skip_lines = false;
        let mut in_head = false;
        let result: Vec<&str> = content_lines
            .into_iter()
            .filter(|line| {
                if line.starts_with(GitUtils::HEAD_MARKER) {
                    in_head = true;
                    skip_lines = true;
                    return false;
                }
                if line.starts_with(GitUtils::BASE_MARKER) {
                    in_head = false;
                    return false;
                }
                if line.starts_with(GitUtils::END_MARKER) {
                    skip_lines = false;
                    in_head = false;
                    return false;
                }
                !skip_lines || in_head
            })
            .collect();

        // Check for erratic conflict markers
        let has_erratic_markers = result.iter().any(|line| {
            line.starts_with(GitUtils::BASE_MARKER)
                || line == &format!("{}\n", GitUtils::CONFLICT_MARKER)
        });

        if has_erratic_markers {
            Err(anyhow::anyhow!("Erratic conflict markers found in file"))
        } else {
            Ok(result)
        }
    }

    /// Apply resolved conflicts back to the repository
    pub fn apply_resolved_conflicts(&self, conflicts: &[ResolvedConflict]) -> Result<()> {
        let conflicts = Self::deduplicate_conflicts(conflicts);

        for conflict in conflicts.iter().rev() {
            println!(
                "Applying resolved conflict for: {}:{} - {}",
                conflict.dedup.conflict.file_path,
                conflict.dedup.conflict.start_line,
                conflict.dedup.model
            );

            // Read the file
            let mut content =
                fs::read_to_string(&conflict.dedup.conflict.file_path).with_context(|| {
                    format!("Failed to read file: {}", conflict.dedup.conflict.file_path)
                })?;

            // Split content into lines
            let mut lines: Vec<String> = content
                .split_inclusive('\n')
                .map(|s| s.to_string())
                .collect();

            // Calculate the line where we want to insert the resolved content
            //print startline and remote start
            let insert_line =
                conflict.dedup.conflict.start_line + conflict.dedup.conflict.remote_start - 1;

            // Insert the resolved content with markers
            let marker_raw = format!("{}{}: ", Self::BASE_MARKER, env!("CARGO_PKG_NAME"));
            let marker = format!("{}{}\n", marker_raw, conflict.dedup.model);
            let current_line = &lines[insert_line];
            if current_line != "=======\n" && !current_line.starts_with(&marker_raw) {
                log::error!("Invalid conflict marker found at line {}", insert_line);
                continue;
            }
            lines.insert(insert_line, marker);
            let resolved_lines: Vec<String> = conflict
                .dedup
                .resolved_version
                .lines()
                .map(|s| s.to_string())
                .collect();
            for (i, line) in resolved_lines.iter().enumerate() {
                lines.insert(insert_line + 1 + i, format!("{}\n", line));
            }

            content = lines.join("");

            // Write back to file
            fs::write(&conflict.dedup.conflict.file_path, content).with_context(|| {
                format!(
                    "Failed to write file: {}",
                    conflict.dedup.conflict.file_path
                )
            })?;
        }

        // Add Assisted-by line to merge message
        self.update_merge_message()?;

        Ok(())
    }

    fn deduplicate_conflicts(conflicts: &[ResolvedConflict]) -> Vec<ResolvedConflict> {
        use std::collections::HashMap;
        let mut map: HashMap<(String, usize), Vec<&ResolvedConflict>> = HashMap::new();

        // Group conflicts by resolved_version and start_line
        for conflict in conflicts {
            map.entry((
                conflict.dedup.resolved_version.clone(),
                conflict.dedup.conflict.start_line,
            ))
            .or_default()
            .push(conflict);
        }

        // For each group, create a new conflict with combined model names
        let mut result = Vec::new();
        for ((resolved_version, _), group) in map {
            let model = group
                .iter()
                .map(|c| c.dedup.model.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            // Use the first conflict in the group as the base
            let base_conflict = group[0];
            result.push(ResolvedConflict {
                dedup: DedupResolvedConflict {
                    conflict: base_conflict.dedup.conflict.clone(),
                    resolved_version,
                    model,
                },
                duration: base_conflict.duration,
                total_tokens: base_conflict.total_tokens,
            });
        }

        // Restore original order
        let mut ordered_result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for original in conflicts {
            let key = (
                &original.dedup.resolved_version,
                original.dedup.conflict.start_line,
            );
            if seen.insert(key)
                && let Some(pos) = result
                    .iter()
                    .position(|r| (&r.dedup.resolved_version, r.dedup.conflict.start_line) == key)
            {
                ordered_result.push(result[pos].clone());
            }
        }

        ordered_result
    }

    /// Get the git root directory
    fn get_git_root(&self) -> Result<String> {
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
        Ok(git_root)
    }

    /// Get the git directory
    fn get_git_dir(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .output()
            .context("Failed to execute git rev-parse")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Git rev-parse failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(git_dir)
    }

    /// Update the git merge message to include Assisted-by line
    fn update_merge_message(&self) -> Result<()> {
        let git_root = self.get_git_root()?;

        let merge_msg_path = if self.in_rebase {
            format!("{}/.git/{}", git_root, Self::REBASE_MESSAGE_FILE)
        } else {
            format!("{}/.git/{}", git_root, Self::MERGE_MSG_FILE)
        };
        let merge_msg_content = match fs::read_to_string(&merge_msg_path) {
            Ok(content) => content,
            Err(_) => {
                println!(
                    "If you use the AI generated code please add \"{}\"",
                    Self::ASSISTED_BY_LINE
                );
                return Ok(());
            }
        };

        if merge_msg_content.contains(Self::ASSISTED_BY_LINE) {
            return Ok(());
        }

        let mut lines: Vec<String> = merge_msg_content
            .split_inclusive('\n')
            .map(|s| s.to_string())
            .collect();

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
        let assisted_line = format!("{}\n", Self::ASSISTED_BY_LINE);
        lines.insert(insert_pos + 1, assisted_line);

        let updated_content = lines.join("");
        fs::write(&merge_msg_path, updated_content).with_context(|| {
            format!("Failed to write updated merge message: {}", merge_msg_path)
        })?;

        println!("Added \"{}\"", Self::ASSISTED_BY_LINE);

        Ok(())
    }

    /// Check if we are currently in a cherry-pick, merge, or rebase state
    pub fn find_commit_hash(&mut self) -> Result<Option<String>> {
        let git_dir = self.get_git_dir()?;

        // Check for cherry-pick, merge, and rebase HEAD files
        let mut head_files = Vec::new();
        for &prefix in &["CHERRY_PICK", "MERGE", "REBASE", "REVERT"] {
            head_files.push((prefix, format!("{}/{}_{}", git_dir, prefix, "HEAD")));
        }

        let mut content: Option<String> = None;
        let mut latest_path: Option<(&str, String)> = None;
        let mut latest_time = std::time::SystemTime::UNIX_EPOCH;

        for (name, path) in head_files {
            if std::path::Path::new(&path).exists() {
                let metadata = std::fs::metadata(&path)
                    .with_context(|| format!("Failed to get metadata for {}", name))?;
                let file_time = metadata
                    .modified()
                    .with_context(|| format!("Failed to get modification time for {}", name))?;

                if file_time > latest_time {
                    latest_time = file_time;
                    latest_path = Some((name, path));
                }
            }
        }

        if let Some((name, path)) = latest_path {
            content = Some(
                std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", name))?
                    .trim()
                    .to_string(),
            );
            // Check if it's a rebase
            if name == "REBASE" {
                // Also check if the rebase message file exists
                let rebase_msg_path = format!("{}/{}", git_dir, Self::REBASE_MESSAGE_FILE);
                if std::path::Path::new(&rebase_msg_path).exists() {
                    self.in_rebase = true;
                }
            }
        }

        Ok(content)
    }

    /// Extract the patch from a specific commit hash
    pub fn extract_diff(&self, commit_hash: &str) -> Result<Option<String>> {
        self.extract_diff_in_dir(commit_hash, None)
    }

    /// Extract the patch from a specific commit hash
    pub fn extract_diff_in_dir(
        &self,
        commit_hash: &str,
        dir: Option<&str>,
    ) -> Result<Option<String>> {
        let context_lines = &format!("-U{}", self.context_lines);
        let mut args = vec![
            "show",
            "--pretty=",
            "--no-color",
            context_lines,
            commit_hash,
        ];
        if let Some(directory) = dir {
            args.splice(0..0, ["-C", directory].iter().cloned());
        }
        let output = Command::new("git")
            .args(&args)
            .output()
            .context("Failed to execute git show")?;

        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let lines: Vec<&str> = stdout.lines().collect();
        let mut result_lines = Vec::new();
        let mut include_line = true;

        for line in lines {
            if line.starts_with("diff --git") {
                result_lines.push(line);
                include_line = false;
            } else if line.starts_with("---") {
                result_lines.push(line);
                include_line = true;
            } else if include_line {
                result_lines.push(line);
            }
        }

        Ok(Some(result_lines.join("\n")))
    }
}

// Local Variables:
// rust-format-on-save: t
// End:
