#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

mod calendar;
mod cli;
mod dates;
mod models;
mod output;

use anyhow::{Context, Result};
use clap::Parser;
use cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let output = calendar::run(cli.command)?;

    if cli.json {
        serde_json::to_writer(std::io::stdout(), &output).context("failed to write JSON output")?;
        println!();
    } else {
        output::print_human_output(&output);
    }

    Ok(())
}
