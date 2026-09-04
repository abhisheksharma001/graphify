//! Argument parsing and dispatch. Every subcommand the engine grows lands here.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use graphify::{assistants, auth, db, jobs, rules, secrets, server, sync, vapi};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
    /// Pull calls from Vapi, apply retention, re-run every rule, then let the patterns
    /// with a model in the loop read what is new — inside `GRAPHIFY_DAILY_CAP_USD`.
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
    /// Re-run every free-mode pattern's rule over its org's calls. Costs nothing: a free
    /// pattern is decided by its rule alone, with no model in the loop.
    Apply,
    /// Run one rule over a file of calls and print the ids it matches, one per line.
    ///
    /// Reads no database and writes none. This is how the brain checks a rule it has just
    /// synthesised against the calls a model labelled, and the engine is the only thing
    /// that gets to say what a rule means.
    RuleCheck {
        /// A JSON rule, as `SynthesizeRule` returns it.
        #[arg(long)]
        rule: PathBuf,
        /// A JSON array of calls: `{id, transcript, ended_reason, ended_group,
        /// transferred, duration_s, tool_calls: [{name, failed}]}`. Only `id` is required.
        #[arg(long)]
        calls: PathBuf,
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
                    org: org.clone(),
                    last,
                    since,
                    base: vapi::DEFAULT_BASE.to_string(),
                    key,
                };
                let report = tokio::runtime::Runtime::new()?.block_on(sync::run(&mut db, &opts))?;
                println!("{report}");

                // Then what the new calls mean: every rule re-run, and after that the
                // patterns with a model in the loop reading what those rules selected.
                // The cap is read here rather than out there, so a `GRAPHIFY_DAILY_CAP_USD`
                // nobody can parse stops the morning at a message instead of at a bill.
                let store = secrets::Secrets::open(secrets::default_key_path())?;
                let daily = sync::daily(
                    &Arc::new(Mutex::new(db)),
                    &store,
                    &sync::DailyOpts {
                        org,
                        brain: jobs::binary_from_env(),
                        cap_usd: sync::daily_cap_from_env()?,
                    },
                )?;
                println!("{daily}");
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
            Command::Apply => {
                let mut db = db::Db::open(db::default_path())?;
                let report = rules::apply(&mut db)?;
                if report.is_empty() {
                    println!("no free-mode pattern has a rule yet");
                }
                for row in &report {
                    println!("{}: {} of {} calls", row.name, row.matched, row.of);
                }
                Ok(())
            }
            Command::RuleCheck { rule, calls } => {
                let checked = rules::validate(&read(&rule)?, &rule.display().to_string())?;
                let calls: Vec<rules::Subject> = serde_json::from_str(&read(&calls)?)
                    .with_context(|| format!("{} is not a JSON array of calls", calls.display()))?;
                for call in calls.iter().filter(|c| rules::matches(&checked, c)) {
                    println!("{}", call.id);
                }
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

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}
