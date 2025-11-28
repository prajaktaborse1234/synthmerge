// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
    /// Path to configuration file
    #[arg(
        short = 'c',
        long = "config",
        default_value = concat!("~/.config/", env!("CARGO_PKG_NAME"), ".yaml")
    )]
    pub config_path: String,

    /// Path to test database file (JSON lines format)
    #[arg(short = 'd', long = "database")]
    pub database_path: String,

    /// Path to checkpoint file
    #[arg(short = 'k', long = "checkpoint")]
    pub checkpoint_path: String,

    /// Checkpoint interval (number of entries between saves)
    #[arg(long = "checkpoint-interval", default_value = "100")]
    pub checkpoint_interval: usize,

    /// Git directories to search for diffs
    #[arg(long = "git-dirs", value_delimiter = ',', required = true)]
    pub git_directories: Vec<String>,

    /// Number of context lines to include around conflict markers
    #[arg(long = "code-context-lines", default_value = "3", value_parser = clap::value_parser!(u32).range(0..))]
    pub code_context_lines: u32,

    /// Number of context lines of the git_diff provided as context
    #[arg(long = "diff-context-lines", default_value = "3", value_parser = clap::value_parser!(u32).range(0..))]
    pub diff_context_lines: u32,

    /// Number of context lines of the patch
    #[arg(long = "patch-context-lines", default_value = "3", value_parser = clap::value_parser!(u32).range(0..))]
    pub patch_context_lines: u32,
}

// Local Variables:
// rust-format-on-save: t
// End:
