#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

mod batch;
mod cache;
mod calendar;
mod calendar_selector;
mod cli;
mod dates;
mod eventkit_bridge;
mod models;
mod output;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Command};
use std::process::ExitCode;

fn main() -> Result<ExitCode> {
    let Cli { json, command } = Cli::parse();

    if let Command::Completions { shell } = command {
        cli::print_completions(shell);
        return Ok(ExitCode::SUCCESS);
    }

    let output = calendar::run(command)?;
    if let Err(error) = cache::update_from_output(&output) {
        eprintln!("warning: failed to update event cache: {error:#}");
    }

    if json {
        serde_json::to_writer(std::io::stdout(), &output).context("failed to write JSON output")?;
        println!();
    } else {
        output::print_human_output(&output);
    }

    if output.has_failures() {
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}
