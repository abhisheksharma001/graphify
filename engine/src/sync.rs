//! `graphify sync`: pull calls from Vapi, extract them into rows, drop what retention no
//! longer covers. This is the first place the client (S-7) and the extractor (S-8) meet.
//!
//! Two bounds decide what gets fetched, and they mean different things. `--last N` is a
//! target size for the org, so rows already stored count against it. `--since DATE` is a
//! range, so it does not. With neither, `since` defaults to the newest `created_at`
//! already stored, which is what makes a re-run cheap.

use crate::assistants;
use crate::db::Db;
use crate::extract::extract;
use crate::jobs;
use crate::now;
use crate::rules;
use crate::secrets::Secrets;
use crate::vapi::{fetch_calls_at, FetchOpts, Retry};
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

/// D-5: retention can be shortened, never lengthened past this.
pub const MAX_KEEP_DAYS: i64 = 14;

/// Target call count when `--last` is not given.
pub const DEFAULT_LAST: usize = 250;

pub struct Opts {
    pub org: String,
    pub last: usize,
    pub since: Option<String>,
    pub base: String,
    pub key: String,
}

/// What one sync did, in the shape the spec prints it.
#[derive(Debug)]
pub struct Report {
    pub org: String,
    pub new: i64,
    pub total: i64,
    pub purged: usize,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "org {}: synced {} new, {} total, purged {}",
            self.org, self.new, self.total, self.purged
        )
    }
}

pub async fn run(db: &mut Db, opts: &Opts) -> Result<Report> {
    let Some(org) = db.org_by_name(&opts.org)? else {
        bail!("no org named {}", opts.org);
    };

    // Checked before anything is fetched: a bad retention setting must not cost a request,
    // and must never reach the DELETE below.
    let keep_days = org.keep_days.unwrap_or(MAX_KEEP_DAYS);
    if keep_days > MAX_KEEP_DAYS {
        bail!(
            "org {} has keep_days = {keep_days}, above the {MAX_KEEP_DAYS}-day retention cap",
            org.name
        );
    }

    // Tools and assistants first, always. `tools.is_transfer` is what lets the extractor
    // recognise a transfer that `endedReason` alone does not name, so a stale tools table
    // silently under-counts transfers on every call written below. The counts are dropped
    // on purpose: this is a precondition of extracting, not something `sync` was asked for.
    assistants::run(
        db,
        &assistants::Opts {
            org: opts.org.clone(),
            base: opts.base.clone(),
            key: opts.key.clone(),
        },
    )
    .await?;

    let stored = db.count_calls(org.id)?;
    let budget = match opts.since {
        Some(_) => opts.last,
        None => opts.last.saturating_sub(stored as usize),
    };
    let since = match &opts.since {
        Some(s) => Some(s.clone()),
        // `max()` over TEXT is lexicographic, which is chronological for the fixed-width
        // UTC instants Vapi returns — the same property the `createdAtLt` cursor rests on.
        None => db.newest_call_created_at(org.id)?,
    };

    if budget > 0 {
        let fetch = FetchOpts {
            last: budget,
            since,
            ..FetchOpts::default()
        };
        let calls = fetch_calls_at(&opts.base, &opts.key, &fetch, Retry::default()).await?;

        let transfer_tools = db.transfer_tool_names(org.id)?;
        let synced_at = now();
        for raw in &calls {
            let (mut call, tools) = extract(raw, org.id, &transfer_tools)?;
            call.synced_at = Some(synced_at.clone());
            db.upsert_call(&call)?;
            db.replace_tool_calls(&call.id, &tools)?;
        }
    }

    // Counted, not assumed: a call that was already stored adds nothing to `new`, however
    // it got here.
    let new = db.count_calls(org.id)? - stored;
    let purged = db.purge_calls(org.id, keep_days, org.max_calls)?;

    Ok(Report {
        org: org.name,
        new,
        total: db.count_calls(org.id)?,
        purged,
    })
}

// ---------------------------------------------------------------------------
// The daily run. Everything above this line is the pull; this is what happens to
// what was pulled.
// ---------------------------------------------------------------------------

/// The default ceiling on what one org's daily runs may spend in a day, in USD. D-8's
/// number, and the one `GRAPHIFY_DAILY_CAP_USD` overrides.
pub const DEFAULT_DAILY_CAP_USD: f64 = 5.0;

/// The environment variable that moves it.
pub const DAILY_CAP_VAR: &str = "GRAPHIFY_DAILY_CAP_USD";

/// What the environment says a day may cost, or [`DEFAULT_DAILY_CAP_USD`].
///
/// A value that will not parse is an error and not a fall back to the default: somebody
/// who wrote `GRAPHIFY_DAILY_CAP_USD=2,50` meant to set a cap, and quietly giving them
/// five dollars instead is the one mistake this number exists to prevent. Zero is allowed
/// and means zero — it is how the daily modes are turned off for a machine without editing
/// every pattern on it.
pub fn daily_cap_from_env() -> Result<f64> {
    let Ok(text) = std::env::var(DAILY_CAP_VAR) else {
        return Ok(DEFAULT_DAILY_CAP_USD);
    };
    let cap: f64 = text
        .trim()
        .parse()
        .with_context(|| format!("{DAILY_CAP_VAR} is {text:?}, which is not a number of dollars"))?;
    if cap < 0.0 || !cap.is_finite() {
        bail!("{DAILY_CAP_VAR} is {text:?}; a daily cap cannot be negative");
    }
    Ok(cap)
}

pub struct DailyOpts {
    pub org: String,
    /// The brain to spawn, as [`jobs::binary_from_env`] found it.
    pub brain: String,
    /// The org's ceiling for the whole day, not for this run. What has already been spent
    /// today comes off it before the brain is told what is left.
    pub cap_usd: f64,
}

/// What one daily run did.
#[derive(Debug)]
pub struct Daily {
    pub org: String,
    /// How many patterns had their rule re-run. Every mode, and free of charge.
    pub applied: usize,
    /// The `jobs` row, when a brain was started. `None` when nothing needed one, which is
    /// the ordinary case for an org that uses free patterns only.
    pub job: Option<i64>,
    /// How that job ended. Printed rather than swallowed: a brain that is not installed
    /// costs nothing, and a line saying it spent nothing would be true and useless.
    pub status: String,
    pub usd: f64,
    /// Why there was nothing to do, when there was nothing to do.
    pub note: Option<String>,
}

impl fmt::Display for Daily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "org {}: applied {} rules", self.org, self.applied)?;
        match (&self.note, self.job) {
            (Some(note), _) => write!(f, ", no daily run: {note}"),
            (None, Some(id)) => write!(
                f,
                ", daily job {id} {} having spent ${:.4}",
                self.status, self.usd
            ),
            (None, None) => Ok(()),
        }
    }
}

/// Re-run every rule in the org, then let the model-backed patterns read what is new.
///
/// The order is the whole of it. In hybrid the rule chooses which calls a model is paid to
/// read, so the rule has to have seen this morning's calls before the reading starts —
/// which is why this runs the rule half itself rather than leaving it to a separate
/// `graphify apply`, and why `sync` is where it lives.
///
/// Three things can stop it before a process is started, and each of them is cheaper than
/// starting one: no cap left for today, no cap at all, and no pattern that would spend.
pub fn daily(db: &Arc<Mutex<Db>>, secrets: &Secrets, opts: &DailyOpts) -> Result<Daily> {
    if opts.cap_usd < 0.0 {
        bail!("a daily cap cannot be negative, and ${:.2} is", opts.cap_usd);
    }

    let (org_id, org_name, applied, wanted, left) = {
        let mut db = lock(db);
        let Some(org) = db.org_by_name(&opts.org)? else {
            bail!("no org named {}", opts.org);
        };
        let applied = rules::apply_org(&mut db, org.id)?.len();
        let wanted = model_backed(&db, org.id)?;
        let left = opts.cap_usd - db.spend_on(&now()[..10], org.id)?;
        (org.id, org.name, applied, wanted, left)
    };

    let stop = |note: &str| {
        Ok(Daily {
            org: org_name.clone(),
            applied,
            job: None,
            status: String::new(),
            usd: 0.0,
            note: Some(note.to_string()),
        })
    };
    if wanted == 0 {
        return stop("no pattern in this org has a model in the loop");
    }
    if left <= 0.0 {
        return stop(&format!(
            "the ${:.2} cap for today is already spent",
            opts.cap_usd
        ));
    }

    // What is left of the day, not the whole cap: two runs on one morning must not be two
    // caps. The brain takes each pattern's own `daily_cap_usd` off this in turn.
    let request = json!({ "org": org_id, "max_usd": left });
    let id = jobs::run_blocking(db, secrets, &opts.brain, jobs::Kind::Daily, org_id, &request)?;
    // The job is finished by the time `run_blocking` returns, so this is the row as it
    // will stay. A failure is reported and not raised: the pull worked, and a cron that
    // treated a provider being down as a failed sync would re-pull the whole org tomorrow.
    let done = lock(db).job(id)?;
    Ok(Daily {
        org: org_name,
        applied,
        job: Some(id),
        status: done
            .as_ref()
            .map_or_else(|| "vanished".to_string(), |j| j.status.clone()),
        usd: done.map_or(0.0, |j| j.cost_usd),
        note: None,
    })
}

/// How many of this org's patterns would spend if a daily run started.
fn model_backed(db: &Db, org_id: i64) -> Result<i64> {
    Ok(db.conn().query_row(
        "SELECT COUNT(*) FROM patterns WHERE org_id = ?1 AND mode IN ('hybrid', 'full')",
        [org_id],
        |r| r.get(0),
    )?)
}

/// The same reasoning `jobs::lock` gives: a poisoned lock means another thread panicked
/// mid-statement, which SQLite survives.
fn lock(db: &Mutex<Db>) -> MutexGuard<'_, Db> {
    db.lock().unwrap_or_else(|e| e.into_inner())
}
