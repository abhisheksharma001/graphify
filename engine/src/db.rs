//! SQLite storage. Opens the file, runs migrations, and writes the two row
//! shapes the sync path produces. No ORM: plain SQL, one statement per helper.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use rusqlite_migration::{Migrations, M};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const INIT: &str = include_str!("../migrations/0001_init.sql");

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
        Migrations::new(vec![M::up(INIT)])
            .to_latest(&mut conn)
            .context("running migrations")?;
        Ok(Self { conn })
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

    /// Store a secret's ciphertext. `last4` is the only part of the value that lands in
    /// clear, and it is `None` for a value too short to give a tail away safely.
    pub fn upsert_secret(
        &self,
        org_id: i64,
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
    pub fn secret(&self, org_id: i64, name: &str) -> Result<Option<(Vec<u8>, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT ciphertext, last4 FROM secrets WHERE org_id = ?1 AND name = ?2",
                params![org_id, name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Enforce retention: drop calls older than `keep_days`, then drop everything past the
    /// newest `max_calls`. Returns how many rows went.
    ///
    /// A call with no `created_at` has an unknown age, so the age sweep leaves it alone
    /// rather than guess it is old. The `max_calls` sweep sorts it last, since unknown
    /// recency is not recency.
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
}
