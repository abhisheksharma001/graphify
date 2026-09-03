//! Argument parsing and dispatch. Every subcommand the engine grows lands here.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use graphify::{db, sync, vapi};

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
    /// Pull calls from Vapi into the local database, then apply retention.
    Sync {
        /// Org to sync, by name.
        #[arg(long)]
        org: String,
        /// How many calls this org should end up holding. Rows already stored count
        /// against it, so a re-run fetches only the shortfall.
        #[arg(long, default_value_t = sync::DEFAULT_LAST)]
        last: usize,
        /// Fetch calls created after this ISO-8601 instant instead of after the newest
        /// one already stored. A range, so stored rows do not count against `--last`.
        #[arg(long)]
        since: Option<String>,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Version => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                Ok(())
            }
            Command::Sync { org, last, since } => {
                // S-11 adds the encrypted store; env wins over it either way, so reading
                // env alone is the whole of this today.
                let key = std::env::var("VAPI_API_KEY")
                    .ok()
                    .filter(|k| !k.trim().is_empty())
                    .context("no Vapi key: set VAPI_API_KEY")?;
                let mut db = db::Db::open(db::default_path())?;
                let opts = sync::Opts {
                    org,
                    last,
                    since,
                    base: vapi::DEFAULT_BASE.to_string(),
                    key,
                };
                let report = tokio::runtime::Runtime::new()?.block_on(sync::run(&mut db, &opts))?;
                println!("{report}");
                Ok(())
            }
        }
    }
}
