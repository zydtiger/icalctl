#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

mod batch;
mod cache;
mod calendar;
mod calendar_selector;
mod cli;
mod config;
mod dates;
mod doctor;
mod eventkit_bridge;
pub mod flightaware;
mod models;
mod output;
mod reminders;
pub mod travel;
mod travel_server;
mod version;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Command, TravelCommand};
use std::process::ExitCode;

fn main() -> Result<ExitCode> {
    let Cli { json, command } = Cli::parse();

    let command = match command {
        Command::Completions { shell } => {
            cli::print_completions(shell);
            return Ok(ExitCode::SUCCESS);
        }
        Command::Travel {
            command: TravelCommand::Serve { calendar_ids },
        } => {
            travel_server::serve(calendar_ids, json)?;
            return Ok(ExitCode::SUCCESS);
        }
        command => command,
    };

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
