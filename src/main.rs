// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use crate::config::Config;
use crate::conflict_resolver::ConflictResolver;
use crate::git_utils::{ContextLines, GitUtils};
use anyhow::Result;
use clap::Parser;

mod api_client;
mod config;
mod conflict_resolver;
mod git_utils;
mod logger;
mod prob;

#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
	default_value = concat!("~/.config/", env!("CARGO_PKG_NAME"), ".yaml")
    )]
    config_path: String,

    /// Number of context lines to include around conflict markers
    #[arg(long = "code-context-lines", default_value = "3", value_parser = clap::value_parser!(u32).range(0..))]
    code_context_lines: u32,

    /// Number of context lines of the git_diff provided as context
    #[arg(long = "diff-context-lines", default_value = "3", value_parser = clap::value_parser!(u32).range(0..))]
    diff_context_lines: u32,

    /// Number of context lines of the patch
    #[arg(long = "patch-context-lines", default_value = "3", value_parser = clap::value_parser!(u32).range(0..))]
    patch_context_lines: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    logger::log_init();
    let args = Args::parse();

    // Load configuration
    let config_path = std::fs::canonicalize(shellexpand::full(&args.config_path)?.as_ref())?;
    let config = Config::load(&config_path)?;

    log::info!("Using config file: {}", args.config_path);

    // Initialize git utilities
    let mut git_utils = GitUtils::new(
        ContextLines {
            code_context_lines: args.code_context_lines,
            diff_context_lines: args.diff_context_lines,
            patch_context_lines: args.patch_context_lines,
        },
        true,
    );

    // Try to cherry-pick with diff3 mode
    let result = git_utils.check_diff3();
    if result.is_err() {
        eprintln!("Diff3 check failed. Run 'git config merge.conflictStyle diff3' to fix this.");
        std::process::exit(1);
    }

    // Check if we're in a cherry-pick and extract commit if needed
    let git_diff = if let Some(commit_hash) = git_utils.find_commit_hash()? {
        log::info!("Extracting diff for commit {}", commit_hash);
        git_utils.extract_diff(&commit_hash)?
    } else {
        None
    };

    // Check if there are conflicts
    let conflicts = git_utils.find_conflicts()?;

    if conflicts.is_empty() {
        println!("No conflicts found.");
        return Ok(());
    }

    println!("Found {} conflicts to resolve", conflicts.len());

    // Resolve conflicts using AI
    let resolver = ConflictResolver::new(
        ContextLines {
            code_context_lines: args.code_context_lines,
            diff_context_lines: args.diff_context_lines,
            patch_context_lines: args.patch_context_lines,
        },
        &config,
        git_diff,
    );
    let resolved_conflicts = resolver.resolve_conflicts(&conflicts).await?.0;

    git_utils.apply_resolved_conflicts(&resolved_conflicts)?;

    Ok(())
}

// Local Variables:
// rust-format-on-save: t
// End:
