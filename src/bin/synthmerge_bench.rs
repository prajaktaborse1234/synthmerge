// SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
// Copyright (C) 2025  Red Hat, Inc.

use anyhow::Result;
use clap::Parser;
use synthmerge::bench::Bench;
use synthmerge::bench_args::Args;
use synthmerge::config::Config;
use synthmerge::logger::log_init;

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
    test.run_test(&config, &entries, args).await?;

    Ok(())
}

// Local Variables:
// rust-format-on-save: t
// End:
