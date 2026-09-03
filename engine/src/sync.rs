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
use crate::now;
use crate::vapi::{fetch_calls_at, FetchOpts, Retry};
use anyhow::{bail, Result};
use std::fmt;

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
