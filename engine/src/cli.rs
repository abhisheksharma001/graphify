//! Argument parsing and dispatch. Every subcommand the engine grows lands here.

use anyhow::Result;
use clap::{Parser, Subcommand};

/// The graphify engine: pulls Vapi calls, stores them, serves the dashboard.
#[derive(Parser)]
#[command(name = "graphify", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print the engine name and version.
    Version,
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Version => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                Ok(())
            }
        }
    }
}
