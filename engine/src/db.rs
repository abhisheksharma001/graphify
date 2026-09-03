//! SQLite storage. Opens the file, runs migrations, and writes the two row
//! shapes the sync path produces. No ORM: plain SQL, one statement per helper.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};
use std::path::{Path, PathBuf};

const INIT: &str = include_str!("../migrations/0001_init.sql");

/// Where the DB lives when the caller does not say: `$GRAPHIFY_DB`, else `data/graphify.db`.
pub fn default_path() -> PathBuf {
    match std::env::var_os("GRAPHIFY_DB") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("data/graphify.db"),
    }
}

/// One org row, as `list_orgs` returns it.
#[derive(Debug)]
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

    pub fn create_org(&self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO orgs (name, created_at) VALUES (?1, datetime('now'))",
            params![name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_orgs(&self) -> Result<Vec<Org>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, provider, keep_days, max_calls, created_at FROM orgs ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Org {
                id: r.get(0)?,
                name: r.get(1)?,
                provider: r.get(2)?,
                keep_days: r.get(3)?,
                max_calls: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
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
}
