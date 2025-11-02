// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use anyhow::Result;
use clap::Parser;
use synthmerge::bench::Bench;
use synthmerge::config::Config;
use synthmerge::logger::log_init;

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

    /// Path to test database file (JSON lines format)
    #[arg(short = 'd', long = "database")]
    database_path: String,

    /// Path to checkpoint file
    #[arg(short = 'k', long = "checkpoint")]
    checkpoint_path: Option<String>,

    /// Checkpoint interval (number of entries between saves)
    #[arg(long = "checkpoint-interval", default_value = "100")]
    checkpoint_interval: usize,

    /// Git directories to search for diffs
    #[arg(long = "git-dirs", value_delimiter = ',')]
    git_directories: Option<Vec<String>>,

    /// Number of context lines to include around conflicts
    #[arg(long = "context-lines", default_value = "3", value_parser = clap::value_parser!(u32).range(1..))]
    context_lines: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    log_init();
    let args = Args::parse();

    // Load configuration
    let config_path = std::fs::canonicalize(shellexpand::full(&args.config_path)?.as_ref())?;
    let config = Config::load(&config_path)?;

    // Load test database
    let db_path = shellexpand::full(&args.database_path)?;
    let db_path = db_path.as_ref();
    let entries = Bench::load_database(db_path)?;

    println!("Loaded {} test entries", entries.len());

    // Create statistics test
    let mut test = Bench::new();

    // Run test
    test.run_test(
        &config,
        &entries,
        args.checkpoint_interval,
        args.checkpoint_path.as_deref(),
        args.git_directories,
        args.context_lines,
    )
    .await?;

    Ok(())
}

// Local Variables:
// rust-format-on-save: t
// End:
