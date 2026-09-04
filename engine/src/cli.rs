//! Argument parsing and dispatch. Every subcommand the engine grows lands here.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use graphify::{assistants, auth, db, secrets, server, sync, vapi};

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
    /// Refresh the org's tools and assistants. `sync` does this first on its own.
    Assistants {
        /// Org to refresh, by name.
        #[arg(long)]
        org: String,
    },
    /// Serve the dashboard and its API on `GRAPHIFY_BIND`, or `127.0.0.1:3737`.
    Serve {
        /// Stay in the terminal instead of opening the dashboard in a browser.
        #[arg(long)]
        no_open: bool,
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
                let mut db = db::Db::open(db::default_path())?;
                let key = vapi_key(&db, &org)?;
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
            Command::Assistants { org } => {
                let db = db::Db::open(db::default_path())?;
                let key = vapi_key(&db, &org)?;
                let opts = assistants::Opts {
                    org,
                    base: vapi::DEFAULT_BASE.to_string(),
                    key,
                };
                let report =
                    tokio::runtime::Runtime::new()?.block_on(assistants::run(&db, &opts))?;
                for name in &report.names {
                    println!("{name}");
                }
                println!("{report}");
                Ok(())
            }
            Command::Serve { no_open } => {
                let db = db::Db::open(db::default_path())?;
                let store = secrets::Secrets::open(secrets::default_key_path())?;
                let app = server::App::new(db, store, auth::Auth::from_env());
                tokio::runtime::Runtime::new()?.block_on(server::serve(
                    app,
                    &server::bind_addr(),
                    !no_open,
                ))
            }
        }
    }
}

/// The org's Vapi key: the environment first, then the encrypted store. This is what S-11
/// deferred to S-12 — until the API existed there was no way to put a key in the store, so
/// there was nothing to read. The environment still wins, so a setup that worked before
/// works unchanged.
fn vapi_key(db: &db::Db, org: &str) -> Result<String> {
    let Some(row) = db.org_by_name(org)? else {
        bail!("no org named {org}");
    };
    let store = secrets::Secrets::open(secrets::default_key_path())?;
    match store.get(db, Some(row.id), "vapi")? {
        Some(key) => Ok(key.expose().to_string()),
        None => bail!("no Vapi key for org {org}: set VAPI_API_KEY or store one via the API"),
    }
}
