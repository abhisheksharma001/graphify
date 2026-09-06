//! SQLite storage. Opens the file, runs migrations, and writes the two row
//! shapes the sync path produces. No ORM: plain SQL, one statement per helper.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use rusqlite_migration::{Migrations, M};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const INIT: &str = include_str!("../migrations/0001_init.sql");
const GLOBAL_SECRETS: &str = include_str!("../migrations/0002_global_secrets.sql");

/// How long an open connection waits for another one to let go before giving up. Long
/// enough to cover any statement this file runs; short enough that a genuinely stuck
/// writer is reported as an error rather than as a request that never returns.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the DB lives when the caller does not say: `$GRAPHIFY_DB`, else `data/graphify.db`.
pub fn default_path() -> PathBuf {
    match std::env::var_os("GRAPHIFY_DB") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("data/graphify.db"),
    }
}

/// One org row, as `list_orgs` returns it. Serialised straight to the API: an org holds
/// no secret, only the name and the retention settings.
#[derive(Debug, serde::Serialize)]
pub struct Org {
    pub id: i64,
    pub name: String,
    pub provider: Option<String>,
    pub keep_days: Option<i64>,
    pub max_calls: Option<i64>,
    pub created_at: Option<String>,
}

/// A `jobs` row, as the API reports one.
///
/// `input`, `output` and `log` stay as the text they were written as. What the brain
/// printed is stored unedited, so a row read back months later says what the brain
/// answered rather than what today's engine would make of it.
#[derive(Debug)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub status: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub cost_usd: f64,
    pub log: String,
    pub created_at: Option<String>,
    pub finished_at: Option<String>,
}

/// A `patterns` row. The four JSON columns come back as the text they were stored as;
/// parsing them is the API's, which is where a column that will not parse can be dropped
/// without taking the row with it.
#[derive(Debug)]
pub struct Pattern {
    pub id: i64,
    pub org_id: Option<i64>,
    pub name: Option<String>,
    pub criterion: Option<String>,
    pub assistant_ids: Option<String>,
    pub plan: Option<String>,
    pub rule: Option<String>,
    pub chart: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub daily_cap_usd: Option<f64>,
    pub sample_size: Option<i64>,
    pub agreement: Option<f64>,
    pub created_at: Option<String>,
}

const PATTERN_COLUMNS: &str = "id, org_id, name, criterion, assistant_ids, plan, rule, chart,
     model, mode, daily_cap_usd, sample_size, agreement, created_at";

fn pattern_row(r: &Row) -> rusqlite::Result<Pattern> {
    Ok(Pattern {
        id: r.get(0)?,
        org_id: r.get(1)?,
        name: r.get(2)?,
        criterion: r.get(3)?,
        assistant_ids: r.get(4)?,
        plan: r.get(5)?,
        rule: r.get(6)?,
        chart: r.get(7)?,
        model: r.get(8)?,
        mode: r.get(9)?,
        daily_cap_usd: r.get(10)?,
        sample_size: r.get(11)?,
        agreement: r.get(12)?,
        created_at: r.get(13)?,
    })
}

/// A `calls` row. Every field Vapi may omit is an `Option`: missing stays NULL, never 0.
#[derive(Debug, Default)]
pub struct Call {
    pub id: String,
    pub org_id: i64,
    pub assistant_id: Option<String>,
    pub assistant_version: Option<String>,
    pub phone_number_id: Option<String>,
    pub call_type: Option<String>,
    pub status: Option<String>,
    pub created_at: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_s: Option<f64>,
    pub ended_reason: Option<String>,
    pub ended_group: Option<String>,
    pub cost: Option<f64>,
    pub cost_stt: Option<f64>,
    pub cost_llm: Option<f64>,
    pub cost_tts: Option<f64>,
    pub cost_vapi: Option<f64>,
    pub cost_transport: Option<f64>,
    pub cost_analysis: Option<f64>,
    pub llm_prompt_tokens: Option<i64>,
    pub llm_completion_tokens: Option<i64>,
    pub llm_cached_tokens: Option<i64>,
    pub tts_characters: Option<i64>,
    pub transferred: Option<bool>,
    pub transfer_destination: Option<String>,
    pub tool_calls: Option<i64>,
    pub tool_failures: Option<i64>,
    pub turns: Option<i64>,
    pub lat_turn_avg_ms: Option<f64>,
    pub lat_turn_p50_ms: Option<f64>,
    pub lat_turn_p95_ms: Option<f64>,
    pub lat_model_avg_ms: Option<f64>,
    pub lat_voice_avg_ms: Option<f64>,
    pub lat_transcriber_avg_ms: Option<f64>,
    pub lat_endpointing_avg_ms: Option<f64>,
    pub turn_latencies: Option<String>,
    pub success_eval: Option<String>,
    pub summary: Option<String>,
    pub structured: Option<String>,
    pub transcript: Option<String>,
    pub recording_url: Option<String>,
    pub slim: Option<String>,
    pub synced_at: Option<String>,
}

/// One `tool_calls` row. The owning call id is passed separately to `replace_tool_calls`.
#[derive(Debug, Default)]
pub struct ToolCall {
    pub name: Option<String>,
    pub seconds_from_start: Option<f64>,
    pub failed: Option<bool>,
    pub arguments: Option<String>,
    pub result_excerpt: Option<String>,
}

/// One `tools` row. `kind` is Vapi's top-level `type`, which is a Rust keyword.
#[derive(Debug, Default)]
pub struct Tool {
    pub id: String,
    pub org_id: i64,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub is_transfer: bool,
    pub fetched_at: Option<String>,
}

/// One `assistants` row. The raw 49 KB assistant never lands; these columns and the
/// structured-data schema are everything the dashboard reads.
#[derive(Debug, Default)]
pub struct Assistant {
    pub id: String,
    pub org_id: i64,
    pub name: Option<String>,
    pub version: Option<String>,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub voice_provider: Option<String>,
    pub transcriber_provider: Option<String>,
    pub transcriber_model: Option<String>,
    pub system_prompt: Option<String>,
    pub prompt_sha256: Option<String>,
    pub first_message: Option<String>,
    pub tool_ids: Option<String>,
    pub structured_schema: Option<String>,
    pub fetched_at: Option<String>,
}

/// The `orgs` columns every lookup selects, in the order `org_row` reads them.
const ORG_COLUMNS: &str = "id, name, provider, keep_days, max_calls, created_at";

fn org_row(r: &Row) -> rusqlite::Result<Org> {
    Ok(Org {
        id: r.get(0)?,
        name: r.get(1)?,
        provider: r.get(2)?,
        keep_days: r.get(3)?,
        max_calls: r.get(4)?,
        created_at: r.get(5)?,
    })
}

pub struct Db {
    conn: Connection,
    /// The file this connection was opened on. Kept because the engine spawns the brain
    /// with `--db PATH`, and a path carried alongside the handle rather than taken from it
    /// is a path that can point somewhere else.
    path: PathBuf,
}

impl Db {
    /// Open (creating parent dirs and the file if needed) and migrate to the latest schema.
    /// Safe to call on an existing file: migrations are tracked in `user_version`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("creating {}", dir.display()))?;
            }
        }
        let mut conn =
            Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        // Two processes share this file the moment anything schedules a sync: the server
        // holds it open all day, and at six the sync opens it again. SQLite's default for
        // a caller that finds the file locked is to fail it immediately with "database is
        // locked" rather than wait, and every write here is one short statement or one
        // short transaction — so waiting is the whole fix.
        conn.busy_timeout(BUSY_TIMEOUT)
            .context("setting the busy timeout")?;
        Migrations::new(vec![M::up(INIT), M::up(GLOBAL_SECRETS)])
            .to_latest(&mut conn)
            .context("running migrations")?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// The database file, which is what a spawned brain is handed as `--db`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The open connection, for `queries`, which is read-only. Writes keep going through
    /// the helpers below so every statement that changes a row is spelled out in one file.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn create_org(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO orgs (name, created_at) VALUES (?1, datetime('now'))",
            params![name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_orgs(&self) -> Result<Vec<Org>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {ORG_COLUMNS} FROM orgs ORDER BY id"))?;
        let rows = stmt.query_map([], org_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Insert the call, or overwrite it wholesale if the id is already stored.
    /// `INSERT OR REPLACE` is safe here because nothing references `calls` by foreign key.
    pub fn upsert_call(&self, c: &Call) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO calls (
                id, org_id, assistant_id, assistant_version, phone_number_id, call_type, status,
                created_at, started_at, ended_at, duration_s, ended_reason, ended_group,
                cost, cost_stt, cost_llm, cost_tts, cost_vapi, cost_transport, cost_analysis,
                llm_prompt_tokens, llm_completion_tokens, llm_cached_tokens, tts_characters,
                transferred, transfer_destination, tool_calls, tool_failures, turns,
                lat_turn_avg_ms, lat_turn_p50_ms, lat_turn_p95_ms, lat_model_avg_ms,
                lat_voice_avg_ms, lat_transcriber_avg_ms, lat_endpointing_avg_ms, turn_latencies,
                success_eval, summary, structured, transcript, recording_url, slim, synced_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28, ?29,
                ?30, ?31, ?32, ?33,
                ?34, ?35, ?36, ?37,
                ?38, ?39, ?40, ?41, ?42, ?43, ?44
            )",
            params![
                c.id,
                c.org_id,
                c.assistant_id,
                c.assistant_version,
                c.phone_number_id,
                c.call_type,
                c.status,
                c.created_at,
                c.started_at,
                c.ended_at,
                c.duration_s,
                c.ended_reason,
                c.ended_group,
                c.cost,
                c.cost_stt,
                c.cost_llm,
                c.cost_tts,
                c.cost_vapi,
                c.cost_transport,
                c.cost_analysis,
                c.llm_prompt_tokens,
                c.llm_completion_tokens,
                c.llm_cached_tokens,
                c.tts_characters,
                c.transferred,
                c.transfer_destination,
                c.tool_calls,
                c.tool_failures,
                c.turns,
                c.lat_turn_avg_ms,
                c.lat_turn_p50_ms,
                c.lat_turn_p95_ms,
                c.lat_model_avg_ms,
                c.lat_voice_avg_ms,
                c.lat_transcriber_avg_ms,
                c.lat_endpointing_avg_ms,
                c.turn_latencies,
                c.success_eval,
                c.summary,
                c.structured,
                c.transcript,
                c.recording_url,
                c.slim,
                c.synced_at,
            ],
        )?;
        Ok(())
    }

    /// Swap a call's tool rows for `rows`, atomically. A re-sync must not double them up.
    pub fn replace_tool_calls(&mut self, call_id: &str, rows: &[ToolCall]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM tool_calls WHERE call_id = ?1", params![call_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO tool_calls
                    (call_id, name, seconds_from_start, failed, arguments, result_excerpt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for t in rows {
                stmt.execute(params![
                    call_id,
                    t.name,
                    t.seconds_from_start,
                    t.failed,
                    t.arguments,
                    t.result_excerpt,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn org_by_name(&self, name: &str) -> Result<Option<Org>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {ORG_COLUMNS} FROM orgs WHERE name = ?1"),
                params![name],
                org_row,
            )
            .optional()?)
    }

    pub fn org_by_id(&self, id: i64) -> Result<Option<Org>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {ORG_COLUMNS} FROM orgs WHERE id = ?1"),
                params![id],
                org_row,
            )
            .optional()?)
    }

    pub fn count_calls(&self, org_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM calls WHERE org_id = ?1",
            params![org_id],
            |r| r.get(0),
        )?)
    }

    /// Newest `created_at` stored for the org, or NULL if it has no calls yet. Used as the
    /// incremental cutoff, so it must be the raw Vapi string, not a reformatted one.
    pub fn newest_call_created_at(&self, org_id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row(
            "SELECT max(created_at) FROM calls WHERE org_id = ?1",
            params![org_id],
            |r| r.get(0),
        )?)
    }

    /// Names of the org's tools whose `type` is `transferCall`. Empty until S-10 fills
    /// `tools`, which only means transfers get spotted by ended reason and destination.
    pub fn transfer_tool_names(&self, org_id: i64) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM tools
              WHERE org_id = ?1 AND is_transfer = 1 AND name IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![org_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
    }

    pub fn upsert_tool(&self, t: &Tool) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tools (id, org_id, name, type, is_transfer, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![t.id, t.org_id, t.name, t.kind, t.is_transfer, t.fetched_at],
        )?;
        Ok(())
    }

    pub fn upsert_assistant(&self, a: &Assistant) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO assistants (
                id, org_id, name, version, model_provider, model, voice_provider,
                transcriber_provider, transcriber_model, system_prompt, prompt_sha256,
                first_message, tool_ids, structured_schema, fetched_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15
             )",
            params![
                a.id,
                a.org_id,
                a.name,
                a.version,
                a.model_provider,
                a.model,
                a.voice_provider,
                a.transcriber_provider,
                a.transcriber_model,
                a.system_prompt,
                a.prompt_sha256,
                a.first_message,
                a.tool_ids,
                a.structured_schema,
                a.fetched_at,
            ],
        )?;
        Ok(())
    }

    /// The pair that decides whether a stored assistant is still current: its version and
    /// the hash of its system prompt. `None` means the assistant is not stored at all.
    pub fn assistant_fingerprint(
        &self,
        id: &str,
    ) -> Result<Option<(Option<String>, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT version, prompt_sha256 FROM assistants WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// One assistant's system prompt, for the org that owns it. `None` means no such
    /// assistant under that org; a stored assistant with no prompt is `Some(None)`.
    ///
    /// Its own helper rather than a column on `queries::assistants`, because that list is
    /// a picker and these prompts run to tens of kilobytes each. The pattern wizard asks
    /// for exactly the one it is about to plan against, and only when told to.
    pub fn assistant_prompt(&self, org_id: i64, id: &str) -> Result<Option<Option<String>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT system_prompt FROM assistants WHERE id = ?1 AND org_id = ?2",
                params![id, org_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Store a secret's ciphertext. `last4` is the only part of the value that lands in
    /// clear, and it is `None` for a value too short to give a tail away safely.
    /// `org_id` is `None` for a key that belongs to the whole install rather than to one
    /// client — the model keys are one account's, not one org's.
    pub fn upsert_secret(
        &self,
        org_id: Option<i64>,
        name: &str,
        ciphertext: &[u8],
        last4: Option<&str>,
        updated_at: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO secrets (org_id, name, ciphertext, last4, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![org_id, name, ciphertext, last4, updated_at],
        )?;
        Ok(())
    }

    /// A secret's ciphertext and its stored tail, or `None` if it was never set.
    ///
    /// `IS`, not `=`: `org_id = NULL` is NULL rather than true, so the global row would
    /// never be found by the query that stored it.
    pub fn secret(
        &self,
        org_id: Option<i64>,
        name: &str,
    ) -> Result<Option<(Vec<u8>, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT ciphertext, last4 FROM secrets WHERE org_id IS ?1 AND name = ?2",
                params![org_id, name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// The org's retention settings. Both are written every time, because both are
    /// nullable and "leave this one alone" and "clear this one" would otherwise be the
    /// same request.
    pub fn set_org_limits(
        &self,
        id: i64,
        keep_days: Option<i64>,
        max_calls: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE orgs SET keep_days = ?2, max_calls = ?3 WHERE id = ?1",
            params![id, keep_days, max_calls],
        )?;
        Ok(())
    }

    /// Enforce retention: drop calls older than `keep_days`, then drop everything past the
    /// newest `max_calls`. Returns how many rows went.
    ///
    /// A call with no `created_at` has an unknown age, so the age sweep leaves it alone
    /// rather than guess it is old. The `max_calls` sweep sorts it last, since unknown
    /// recency is not recency.
    /// The saved dashboard layout for this org, as the JSON it was written as. NULL when
    /// nothing has been saved, which is not the same thing as a layout with no charts in
    /// it: the first says "the reader has never chosen", the second says "the reader chose
    /// none of them".
    pub fn dashboard(&self, org_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT config FROM dashboard WHERE org_id = ?1",
                params![org_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Replaces the layout outright. It is one preference, not a set of them, so there is
    /// nothing here to merge.
    pub fn set_dashboard(&self, org_id: i64, config: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO dashboard (org_id, config) VALUES (?1, ?2)
               ON CONFLICT(org_id) DO UPDATE SET config = excluded.config",
            params![org_id, config],
        )?;
        Ok(())
    }

    pub fn purge_calls(
        &mut self,
        org_id: i64,
        keep_days: i64,
        max_calls: Option<i64>,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut gone = tx.execute(
            "DELETE FROM calls
              WHERE org_id = ?1 AND created_at IS NOT NULL
                AND julianday(created_at) < julianday('now', ?2)",
            params![org_id, format!("-{keep_days} days")],
        )?;
        if let Some(max) = max_calls {
            gone += tx.execute(
                "DELETE FROM calls
                  WHERE org_id = ?1 AND id NOT IN (
                        SELECT id FROM calls WHERE org_id = ?1
                         ORDER BY created_at DESC LIMIT ?2)",
                params![org_id, max],
            )?;
        }
        tx.execute(
            "DELETE FROM tool_calls WHERE call_id NOT IN (SELECT id FROM calls)",
            [],
        )?;
        tx.commit()?;
        Ok(gone)
    }

    /// Start a job, in whatever state its kind begins in. Returns the new id.
    ///
    /// `input` holds the whole request, the org included: `jobs` has no org column and the
    /// spend a finished job books is keyed by one, so it has to be recoverable from the row.
    pub fn create_job(&self, kind: &str, status: &str, input: &str, created_at: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO jobs (kind, status, input, cost_usd, log, created_at)
             VALUES (?1, ?2, ?3, 0, '', ?4)",
            params![kind, status, input, created_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn job(&self, id: i64) -> Result<Option<Job>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, kind, status, input, output, cost_usd, log, created_at, finished_at
                   FROM jobs WHERE id = ?1",
                params![id],
                |r| {
                    Ok(Job {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        status: r.get(2)?,
                        input: r.get(3)?,
                        output: r.get(4)?,
                        cost_usd: r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                        log: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        created_at: r.get(7)?,
                        finished_at: r.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_job_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = ?2 WHERE id = ?1",
            params![id, status],
        )?;
        Ok(())
    }

    /// Append one line to a job's log, as it arrives. A job that is still running is a job
    /// somebody is watching, and a log written only at the end is no use to them.
    pub fn append_job_log(&self, id: i64, line: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET log = COALESCE(log, '') || ?2 WHERE id = ?1",
            params![id, format!("{line}\n")],
        )?;
        Ok(())
    }

    /// Close a job out and book what it cost, in one transaction.
    ///
    /// The two statements are one fact. A job that reads `done` with a cost on it is a
    /// claim that the org's ledger has already counted that money, and `sync` sizes the
    /// day's remaining budget by subtracting that ledger from the cap — so a row that
    /// closes without its spend landing is a hard cap quietly raised by the amount that
    /// went missing. Ordering the writes was not enough, because the first of them can
    /// fail and the second still run. Inside a transaction they move together or not at
    /// all, and the invariant holds without anyone having to remember it.
    ///
    /// The only helper here that runs more than one statement, for that reason.
    pub fn finish_job(
        &self,
        id: i64,
        status: &str,
        output: Option<&str>,
        cost_usd: f64,
        org_id: i64,
        finished_at: &str,
    ) -> Result<()> {
        // The ledger is keyed by day and the row by the instant, and they are the same
        // moment: taking the date off the timestamp is what keeps them from being two.
        let day = finished_at.get(..10).unwrap_or(finished_at);
        // Unchecked because the borrow checker cannot see what is true here: every caller
        // holds the `Db` behind a mutex and nothing else in the tree opens a transaction,
        // so there is no nesting for the checked form to prevent.
        let tx = self.conn.unchecked_transaction()?;
        if cost_usd > 0.0 {
            tx.execute(
                "INSERT INTO spend (day, org_id, usd) VALUES (?1, ?2, ?3)
                   ON CONFLICT(day, org_id) DO UPDATE SET usd = usd + excluded.usd",
                params![day, org_id, cost_usd],
            )?;
        }
        tx.execute(
            "UPDATE jobs SET status = ?2, output = ?3, cost_usd = ?4, finished_at = ?5
              WHERE id = ?1",
            params![id, status, output, cost_usd, finished_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// How many jobs are still holding a subprocess. Counted before another is started:
    /// a job waiting for its go holds a parked interpreter, and a wizard clicked ten times
    /// should be refused rather than answered with ten of them.
    /// The two statuses are the caller's to name: which of them mean a process is still
    /// alive is `jobs`', not this file's.
    pub fn live_jobs(&self, running: &str, waiting: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE status = ?1 OR status = ?2",
            params![running, waiting],
            |r| r.get(0),
        )?)
    }

    /// Close out every job that a dead process left mid-flight. Called once at startup:
    /// the children died with the engine, so a row still claiming to be running is a row
    /// about a process that no longer exists.
    pub fn abandon_live_jobs(
        &self,
        running: &str,
        waiting: &str,
        status: &str,
        finished_at: &str,
    ) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE jobs SET status = ?3, finished_at = ?4 WHERE status = ?1 OR status = ?2",
            params![running, waiting, status, finished_at],
        )?)
    }

    /// Book money against an org on a day, added to whatever is already there: the cap in
    /// D-8 is a day's total, not a job's. A job's own cost does not come through here —
    /// `finish_job` books it in the same transaction that closes the row, because the two
    /// are one fact. This is the plain entry point, for a ledger written on its own.
    pub fn add_spend(&self, day: &str, org_id: i64, usd: f64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO spend (day, org_id, usd) VALUES (?1, ?2, ?3)
               ON CONFLICT(day, org_id) DO UPDATE SET usd = usd + excluded.usd",
            params![day, org_id, usd],
        )?;
        Ok(())
    }

    /// What has been spent on this org today, for the cap and for the wizard to show.
    pub fn spend_on(&self, day: &str, org_id: i64) -> Result<f64> {
        Ok(self
            .conn
            .query_row(
                "SELECT usd FROM spend WHERE day = ?1 AND org_id = ?2",
                params![day, org_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0.0))
    }

    pub fn list_patterns(&self, org_id: i64) -> Result<Vec<Pattern>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PATTERN_COLUMNS} FROM patterns WHERE org_id = ?1 ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![org_id], pattern_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn pattern(&self, id: i64) -> Result<Option<Pattern>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {PATTERN_COLUMNS} FROM patterns WHERE id = ?1"),
                params![id],
                pattern_row,
            )
            .optional()?)
    }

    /// The three settings the analyst owns: what the pattern matches, whether a model is
    /// in the loop, and how much that model may spend in a day. All three are written every
    /// time, for the reason `set_org_limits` gives — with nullable columns, "leave this one
    /// alone" and "clear this one" are otherwise the same request.
    pub fn set_pattern_rule(
        &self,
        id: i64,
        rule: Option<&str>,
        mode: &str,
        daily_cap_usd: f64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE patterns SET rule = ?2, mode = ?3, daily_cap_usd = ?4 WHERE id = ?1",
            params![id, rule, mode, daily_cap_usd],
        )?;
        Ok(())
    }

    /// Replace one pattern's rule-sourced matches.
    ///
    /// `source='rule'` rows are derived: they are whatever the rule says today, so a
    /// re-run replaces them outright rather than merging. `source='llm'` rows are a
    /// model's answers, paid for once, and this never touches them.
    pub fn replace_rule_matches(&mut self, pattern_id: i64, call_ids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM pattern_matches WHERE pattern_id = ?1 AND source = 'rule'",
            params![pattern_id],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO pattern_matches (pattern_id, call_id, source)
                 VALUES (?1, ?2, 'rule')",
            )?;
            for id in call_ids {
                stmt.execute(params![pattern_id, id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
