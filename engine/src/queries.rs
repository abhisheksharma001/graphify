//! Every read the API makes, and the filter set they all share. No ORM: plain SQL, one
//! statement per helper, exactly as `db.rs` does it.
//!
//! Two rules run through this file. A missing number stays missing — a bucket with no
//! priced call reports a NULL cost, never a zero, because 0 and "we don't know" are
//! different answers to "what did this hour cost". And an unknown filter key is an error,
//! not a shrug: a typo in `assistant_id` that silently returned the whole org would be a
//! wrong chart nobody could spot.

use crate::db::Db;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Row};
use serde::Serialize;
use std::collections::BTreeMap;

/// How many calls `/api/calls` returns when the caller names no `last`. A page, not the
/// whole retention window: the list is something a person scrolls.
const DEFAULT_CALL_LIMIT: usize = 200;

/// Longest span that still gets hourly buckets. Above it the chart would be unreadable
/// and the query pointlessly wide.
const HOURLY_MAX: Duration = Duration::days(2);

/// The filters every endpoint accepts. All optional: with none of them set the answer is
/// the whole database, which is the honest default for a single-tenant local dashboard.
#[derive(Debug, Default, PartialEq)]
pub struct Filters {
    pub org: Option<i64>,
    /// Repeatable. Several ids mean "any of these", not "all of these".
    pub assistant_ids: Vec<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub window: Option<String>,
    pub last: Option<usize>,
    pub ended_group: Option<String>,
    pub call_id: Option<String>,
    pub tool_failed: Option<bool>,
    pub transferred: Option<bool>,
}

impl Filters {
    /// Parse a raw query string. Unknown keys are rejected rather than ignored.
    pub fn from_query(query: &str) -> Result<Self> {
        let mut f = Filters::default();
        for (k, v) in form_urlencoded::parse(query.as_bytes()) {
            let v = v.trim().to_string();
            if v.is_empty() {
                // `?org=` is how a browser sends "no choice made". Treat it as absent
                // rather than as an org named the empty string.
                continue;
            }
            match k.as_ref() {
                "org" => f.org = Some(v.parse().context("org must be an org id")?),
                "assistant_id" => f.assistant_ids.push(v),
                "since" => f.since = Some(v),
                "until" => f.until = Some(v),
                "window" => f.window = Some(v),
                "last" => f.last = Some(v.parse().context("last must be a whole number")?),
                "ended_group" => f.ended_group = Some(v),
                "call_id" => f.call_id = Some(v),
                "tool_failed" => f.tool_failed = Some(flag(&v)?),
                "transferred" => f.transferred = Some(flag(&v)?),
                other => bail!("unknown filter {other}"),
            }
        }
        // Parsed here so a bad window is a 400 on the filter, not a surprise mid-query.
        f.span()?;
        Ok(f)
    }

    /// The window as a duration, if one was given.
    fn span(&self) -> Result<Option<Duration>> {
        self.window.as_deref().map(parse_window).transpose()
    }

    /// The lower bound actually applied: an explicit `since` if there is one, else the
    /// start of the window. Both may be set; the explicit instant is the more specific
    /// answer, so it wins.
    fn floor(&self, now: DateTime<Utc>) -> Result<Option<String>> {
        if self.since.is_some() {
            return Ok(self.since.clone());
        }
        Ok(self.span()?.map(|d| stamp(now - d)))
    }
}

/// `1`, `true` and `yes` are true; `0`, `false` and `no` are false. Nothing else, so a
/// `tool_failed=maybe` fails loudly instead of quietly meaning false.
fn flag(v: &str) -> Result<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        other => bail!("expected true or false, got {other}"),
    }
}

/// `5h`, `7d`, `36h`. A bare number is not accepted: the unit is the whole point.
fn parse_window(w: &str) -> Result<Duration> {
    let (n, unit) = w.split_at(w.len().saturating_sub(1));
    let n: i64 = n.parse().with_context(|| format!("bad window {w}"))?;
    if n <= 0 {
        bail!("window {w} must be positive");
    }
    match unit {
        "h" => Ok(Duration::hours(n)),
        "d" => Ok(Duration::days(n)),
        _ => bail!("window {w} must end in h or d"),
    }
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The `WHERE` for the filter set, plus its parameters in order.
struct Cond {
    sql: String,
    params: Vec<Sql>,
}

fn conditions(f: &Filters, floor: Option<&str>) -> Cond {
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<Sql> = Vec::new();

    if let Some(org) = f.org {
        parts.push("org_id = ?".into());
        params.push(Sql::Integer(org));
    }
    if !f.assistant_ids.is_empty() {
        let holes = vec!["?"; f.assistant_ids.len()].join(", ");
        parts.push(format!("assistant_id IN ({holes})"));
        params.extend(f.assistant_ids.iter().map(|s| Sql::Text(s.clone())));
    }
    // String comparison is chronological for the fixed-width UTC instants Vapi returns,
    // the same property the sync cursor rests on.
    if let Some(since) = floor {
        parts.push("created_at >= ?".into());
        params.push(Sql::Text(since.to_string()));
    }
    if let Some(until) = &f.until {
        parts.push("created_at < ?".into());
        params.push(Sql::Text(until.clone()));
    }
    if let Some(group) = &f.ended_group {
        parts.push("ended_group = ?".into());
        params.push(Sql::Text(group.clone()));
    }
    if let Some(id) = &f.call_id {
        parts.push("id = ?".into());
        params.push(Sql::Text(id.clone()));
    }
    if let Some(failed) = f.tool_failed {
        // A call with no tool calls has NULL failures, which is neither "had one" nor
        // "had none" — leave it out of both answers rather than count it as zero.
        parts.push(if failed { "tool_failures > 0" } else { "tool_failures = 0" }.into());
    }
    if let Some(t) = f.transferred {
        parts.push("transferred = ?".into());
        params.push(Sql::Integer(i64::from(t)));
    }

    let sql = if parts.is_empty() {
        "1 = 1".to_string()
    } else {
        parts.join(" AND ")
    };
    Cond { sql, params }
}

/// The filtered call set, newest first, as a CTE the aggregates below select from.
/// `LIMIT -1` is SQLite for "no limit", so `last` and its absence take the same path.
fn selection(f: &Filters, floor: Option<&str>, limit: i64) -> (String, Vec<Sql>) {
    let cond = conditions(f, floor);
    let sql = format!(
        "WITH sel AS (SELECT * FROM calls WHERE {} ORDER BY created_at DESC LIMIT ?)",
        cond.sql
    );
    let mut params = cond.params;
    params.push(Sql::Integer(limit));
    (sql, params)
}

/// One row of `/api/calls`: enough for the table, without the transcript or the slim blob.
#[derive(Debug, Serialize)]
pub struct CallRow {
    pub id: String,
    pub org_id: Option<i64>,
    pub assistant_id: Option<String>,
    pub assistant_name: Option<String>,
    pub created_at: Option<String>,
    pub duration_s: Option<f64>,
    pub ended_reason: Option<String>,
    pub ended_group: Option<String>,
    pub cost: Option<f64>,
    pub transferred: Option<bool>,
    pub tool_calls: Option<i64>,
    pub tool_failures: Option<i64>,
    pub turns: Option<i64>,
    pub lat_turn_p50_ms: Option<f64>,
    pub lat_turn_p95_ms: Option<f64>,
    pub success_eval: Option<String>,
    pub summary: Option<String>,
}

const CALL_COLUMNS: &str = "sel.id, sel.org_id, sel.assistant_id, a.name, sel.created_at,
    sel.duration_s, sel.ended_reason, sel.ended_group, sel.cost, sel.transferred,
    sel.tool_calls, sel.tool_failures, sel.turns, sel.lat_turn_p50_ms, sel.lat_turn_p95_ms,
    sel.success_eval, sel.summary";

fn call_row(r: &Row) -> rusqlite::Result<CallRow> {
    Ok(CallRow {
        id: r.get(0)?,
        org_id: r.get(1)?,
        assistant_id: r.get(2)?,
        assistant_name: r.get(3)?,
        created_at: r.get(4)?,
        duration_s: r.get(5)?,
        ended_reason: r.get(6)?,
        ended_group: r.get(7)?,
        cost: r.get(8)?,
        transferred: r.get(9)?,
        tool_calls: r.get(10)?,
        tool_failures: r.get(11)?,
        turns: r.get(12)?,
        lat_turn_p50_ms: r.get(13)?,
        lat_turn_p95_ms: r.get(14)?,
        success_eval: r.get(15)?,
        summary: r.get(16)?,
    })
}

pub fn calls(db: &Db, f: &Filters) -> Result<Vec<CallRow>> {
    let floor = f.floor(Utc::now())?;
    let limit = f.last.unwrap_or(DEFAULT_CALL_LIMIT) as i64;
    let (cte, params) = selection(f, floor.as_deref(), limit);
    let sql = format!(
        "{cte} SELECT {CALL_COLUMNS} FROM sel
           LEFT JOIN assistants a ON a.id = sel.assistant_id
          ORDER BY sel.created_at DESC"
    );
    let conn = db.conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params), call_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// One tool invocation on a call, for the drawer.
#[derive(Debug, Serialize)]
pub struct ToolCallRow {
    pub name: Option<String>,
    pub seconds_from_start: Option<f64>,
    pub failed: Option<bool>,
    pub arguments: Option<String>,
    pub result_excerpt: Option<String>,
}

/// Everything the call drawer draws. `slim` is the trimmed Vapi payload, passed through
/// as parsed JSON so the browser does not have to unwrap a string containing JSON.
#[derive(Debug, Serialize)]
pub struct CallDetail {
    #[serde(flatten)]
    pub row: CallRow,
    pub status: Option<String>,
    pub call_type: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub transfer_destination: Option<String>,
    pub cost_stt: Option<f64>,
    pub cost_llm: Option<f64>,
    pub cost_tts: Option<f64>,
    pub cost_vapi: Option<f64>,
    pub cost_transport: Option<f64>,
    pub cost_analysis: Option<f64>,
    pub lat_turn_avg_ms: Option<f64>,
    pub transcript: Option<String>,
    /// The URL only. D-3: audio is never downloaded or stored.
    pub recording_url: Option<String>,
    pub structured: Option<serde_json::Value>,
    pub slim: Option<serde_json::Value>,
    pub tool_call_rows: Vec<ToolCallRow>,
}

pub fn call(db: &Db, id: &str) -> Result<Option<CallDetail>> {
    let conn = db.conn();
    let found = conn
        .query_row(
            &format!(
                "SELECT {CALL_COLUMNS}, sel.status, sel.call_type, sel.started_at, sel.ended_at,
                        sel.transfer_destination, sel.cost_stt, sel.cost_llm, sel.cost_tts,
                        sel.cost_vapi, sel.cost_transport, sel.cost_analysis,
                        sel.lat_turn_avg_ms, sel.transcript, sel.recording_url,
                        sel.structured, sel.slim
                   FROM calls sel
                   LEFT JOIN assistants a ON a.id = sel.assistant_id
                  WHERE sel.id = ?1"
            ),
            [id],
            |r| {
                Ok(CallDetail {
                    row: call_row(r)?,
                    status: r.get(17)?,
                    call_type: r.get(18)?,
                    started_at: r.get(19)?,
                    ended_at: r.get(20)?,
                    transfer_destination: r.get(21)?,
                    cost_stt: r.get(22)?,
                    cost_llm: r.get(23)?,
                    cost_tts: r.get(24)?,
                    cost_vapi: r.get(25)?,
                    cost_transport: r.get(26)?,
                    cost_analysis: r.get(27)?,
                    lat_turn_avg_ms: r.get(28)?,
                    transcript: r.get(29)?,
                    recording_url: r.get(30)?,
                    structured: json_column(r.get::<_, Option<String>>(31)?),
                    slim: json_column(r.get::<_, Option<String>>(32)?),
                    tool_call_rows: Vec::new(),
                })
            },
        )
        .optional_row()?;

    let Some(mut detail) = found else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT name, seconds_from_start, failed, arguments, result_excerpt
           FROM tool_calls WHERE call_id = ?1 ORDER BY seconds_from_start",
    )?;
    let rows = stmt.query_map([id], |r| {
        Ok(ToolCallRow {
            name: r.get(0)?,
            seconds_from_start: r.get(1)?,
            failed: r.get(2)?,
            arguments: r.get(3)?,
            result_excerpt: r.get(4)?,
        })
    })?;
    detail.tool_call_rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(detail))
}

/// A stored JSON column, parsed. Unparseable text is dropped rather than passed on as a
/// string: the field is typed JSON, and half-JSON in a typed field is worse than nothing.
fn json_column(raw: Option<String>) -> Option<serde_json::Value> {
    serde_json::from_str(&raw?).ok()
}

/// One assistant, as `/api/assistants` lists them. The system prompt is deliberately not
/// here: the list is a picker, and the prompts run to tens of kilobytes each.
#[derive(Debug, Serialize)]
pub struct AssistantRow {
    pub id: String,
    pub org_id: Option<i64>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub voice_provider: Option<String>,
    pub transcriber_provider: Option<String>,
    pub transcriber_model: Option<String>,
    pub prompt_sha256: Option<String>,
    pub first_message: Option<String>,
    pub tool_ids: Option<serde_json::Value>,
    pub structured_schema: Option<serde_json::Value>,
    pub fetched_at: Option<String>,
}

pub fn assistants(db: &Db, org: Option<i64>) -> Result<Vec<AssistantRow>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, org_id, name, version, model_provider, model, voice_provider,
                transcriber_provider, transcriber_model, prompt_sha256, first_message,
                tool_ids, structured_schema, fetched_at
           FROM assistants
          WHERE (?1 IS NULL OR org_id = ?1)
          ORDER BY name IS NULL, name, id",
    )?;
    let rows = stmt.query_map([org], |r| {
        Ok(AssistantRow {
            id: r.get(0)?,
            org_id: r.get(1)?,
            name: r.get(2)?,
            version: r.get(3)?,
            model_provider: r.get(4)?,
            model: r.get(5)?,
            voice_provider: r.get(6)?,
            transcriber_provider: r.get(7)?,
            transcriber_model: r.get(8)?,
            prompt_sha256: r.get(9)?,
            first_message: r.get(10)?,
            tool_ids: json_column(r.get::<_, Option<String>>(11)?),
            structured_schema: json_column(r.get::<_, Option<String>>(12)?),
            fetched_at: r.get(13)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The numbers a bucket, an assistant, or the whole selection carries. Counts are counts,
/// so zero is a real answer; every money, duration and latency figure is nullable, so an
/// hour that priced nothing reports nothing.
#[derive(Debug, Default, Serialize)]
pub struct Totals {
    pub calls: i64,
    pub tool_failures: Option<i64>,
    pub transfers: Option<i64>,
    pub cost: Option<f64>,
    pub duration_avg: Option<f64>,
    pub latency_p50: Option<f64>,
    pub latency_p95: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Bucket {
    /// The instant the bucket starts, UTC.
    pub bucket: String,
    #[serde(flatten)]
    pub totals: Totals,
}

#[derive(Debug, Serialize)]
pub struct ByAssistant {
    pub assistant_id: Option<String>,
    pub name: Option<String>,
    #[serde(flatten)]
    pub totals: Totals,
}

#[derive(Debug, Serialize)]
pub struct Stats {
    pub by_ended_group: BTreeMap<String, i64>,
    pub by_ended_reason: BTreeMap<String, i64>,
    /// One entry per bucket across the whole span, including the empty ones: a gap in a
    /// line chart has to be drawn as a gap, not skipped over.
    pub per_bucket: Vec<Bucket>,
    /// Bucket size, `1h` or `1d`, so the chart can label its axis without guessing.
    pub bucket_size: String,
    pub tool_failures_by_name: BTreeMap<String, i64>,
    pub by_assistant: Vec<ByAssistant>,
    pub success_eval_counts: BTreeMap<String, i64>,
    /// How many calls carry each top-level key of `analysis.structuredData`. This is what
    /// tells the UI which structured columns are worth offering.
    pub structured_keys: BTreeMap<String, i64>,
    pub totals: Totals,
}

/// The per-call numbers the buckets are built from, before they are bucketed.
struct Point {
    created_at: Option<String>,
    duration_s: Option<f64>,
    cost: Option<f64>,
    tool_failures: Option<i64>,
    transferred: Option<bool>,
    p50: Option<f64>,
    p95: Option<f64>,
}

pub fn stats(db: &Db, f: &Filters) -> Result<Stats> {
    let now = Utc::now();
    let floor = f.floor(now)?;
    let limit = f.last.map_or(-1, |n| n as i64);
    let (cte, params) = selection(f, floor.as_deref(), limit);
    let conn = db.conn();

    // `skip_null` says what a NULL in this column means. For `ended_group` it means the
    // call has not ended, which `ended_reason::group(None)` already calls "unknown", so it
    // gets a bucket. For `success_eval` it means no evaluation ran, which is not a verdict
    // and must not sit beside the real ones as if it were.
    let counts = |column: &str, skip_null: bool| -> Result<BTreeMap<String, i64>> {
        let key = if skip_null {
            column.to_string()
        } else {
            format!("coalesce({column}, 'unknown')")
        };
        let filter = if skip_null {
            format!("WHERE {column} IS NOT NULL")
        } else {
            String::new()
        };
        let sql = format!("{cte} SELECT {key}, count(*) FROM sel {filter} GROUP BY 1");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.clone()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?)
    };

    let by_ended_group = counts("ended_group", false)?;
    let by_ended_reason = counts("ended_reason", false)?;
    let success_eval_counts = counts("success_eval", true)?;

    let tool_failures_by_name = {
        let sql = format!(
            "{cte} SELECT t.name, count(*) FROM tool_calls t JOIN sel ON t.call_id = sel.id
              WHERE t.failed = 1 AND t.name IS NOT NULL GROUP BY 1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.clone()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?
    };

    let structured_keys = {
        let sql = format!("{cte} SELECT structured FROM sel WHERE structured IS NOT NULL");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.clone()), |r| {
            r.get::<_, String>(0)
        })?;
        let mut keys: BTreeMap<String, i64> = BTreeMap::new();
        for raw in rows {
            let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&raw?) else {
                continue;
            };
            // A key present but null is a key the assistant was asked for and did not
            // fill. That is not a column worth offering, so it does not count.
            for k in map.iter().filter(|(_, v)| !v.is_null()).map(|(k, _)| k) {
                *keys.entry(k.clone()).or_default() += 1;
            }
        }
        keys
    };

    let by_assistant = {
        let sql = format!(
            "{cte} SELECT sel.assistant_id, a.name, count(*), sum(sel.tool_failures),
                          sum(sel.transferred), sum(sel.cost), avg(sel.duration_s)
               FROM sel LEFT JOIN assistants a ON a.id = sel.assistant_id
              GROUP BY 1, 2 ORDER BY 3 DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.clone()), |r| {
            Ok(ByAssistant {
                assistant_id: r.get(0)?,
                name: r.get(1)?,
                totals: Totals {
                    calls: r.get(2)?,
                    tool_failures: r.get(3)?,
                    transfers: r.get(4)?,
                    cost: r.get(5)?,
                    duration_avg: r.get(6)?,
                    // Percentiles do not come out of a GROUP BY; the per-assistant chart
                    // reads counts and cost, and the latency chart is the bucket series.
                    latency_p50: None,
                    latency_p95: None,
                },
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let points = {
        let sql = format!(
            "{cte} SELECT created_at, duration_s, cost, tool_failures, transferred,
                          lat_turn_p50_ms, lat_turn_p95_ms FROM sel"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.clone()), |r| {
            Ok(Point {
                created_at: r.get(0)?,
                duration_s: r.get(1)?,
                cost: r.get(2)?,
                tool_failures: r.get(3)?,
                transferred: r.get(4)?,
                p50: r.get(5)?,
                p95: r.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    // The span the caller asked for, however they asked. A `since` two hours back
    // deserves hourly buckets just as much as `window=2h` does.
    let span = match f.span()? {
        Some(d) => Some(d),
        None => floor.as_deref().and_then(parse_instant).map(|start| {
            f.until.as_deref().and_then(parse_instant).unwrap_or(now) - start
        }),
    };
    let hourly = span.is_some_and(|d| d <= HOURLY_MAX);
    let per_bucket = bucketed(&points, hourly, floor.as_deref(), f.until.as_deref(), now);
    let totals = totals(points.iter());

    Ok(Stats {
        by_ended_group,
        by_ended_reason,
        per_bucket,
        bucket_size: if hourly { "1h" } else { "1d" }.to_string(),
        tool_failures_by_name,
        by_assistant,
        success_eval_counts,
        structured_keys,
        totals,
    })
}

/// Sum a set of calls. A total stays NULL until some call contributes to it, so an hour
/// in which nothing was priced reports no cost rather than a cost of zero. `calls` is the
/// exception: it is a count, and none is a number.
fn totals<'a>(points: impl Iterator<Item = &'a Point>) -> Totals {
    let mut t = Totals::default();
    let (mut p50, mut p95) = (Vec::new(), Vec::new());
    // Averaged over the calls that carried a duration, not over every call: a call still
    // running has no duration to average in.
    let (mut duration_sum, mut durations) = (0.0, 0);

    for p in points {
        t.calls += 1;
        add_i64(&mut t.tool_failures, p.tool_failures);
        add_i64(&mut t.transfers, p.transferred.map(i64::from));
        add_f64(&mut t.cost, p.cost);
        if let Some(v) = p.duration_s {
            duration_sum += v;
            durations += 1;
        }
        if let Some(v) = p.p50 {
            p50.push(v);
        }
        if let Some(v) = p.p95 {
            p95.push(v);
        }
    }

    if durations > 0 {
        t.duration_avg = Some(duration_sum / f64::from(durations));
    }
    p50.sort_by(f64::total_cmp);
    p95.sort_by(f64::total_cmp);
    t.latency_p50 = percentile(&p50, 0.50);
    t.latency_p95 = percentile(&p95, 0.95);
    t
}

fn add_i64(acc: &mut Option<i64>, v: Option<i64>) {
    if let Some(v) = v {
        *acc = Some(acc.unwrap_or(0) + v);
    }
}

fn add_f64(acc: &mut Option<f64>, v: Option<f64>) {
    if let Some(v) = v {
        *acc = Some(acc.unwrap_or(0.0) + v);
    }
}

/// Nearest-rank, the same rule `extract` uses for a call's own turns.
///
/// These are percentiles over per-call percentiles, not over raw turns: the raw turn
/// latencies live in each call's `turn_latencies` blob, and re-parsing every one of them
/// on every dashboard refresh would cost far more than the precision is worth.
fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank - 1).copied()
}

/// Group the points into buckets and fill the gaps between them.
fn bucketed(
    points: &[Point],
    hourly: bool,
    floor: Option<&str>,
    until: Option<&str>,
    now: DateTime<Utc>,
) -> Vec<Bucket> {
    let mut by_key: BTreeMap<String, Vec<&Point>> = BTreeMap::new();
    let mut earliest: Option<DateTime<Utc>> = None;
    let mut latest: Option<DateTime<Utc>> = None;

    for p in points {
        // A call with no `created_at` cannot be placed on a time axis. It still counts in
        // `totals`; it just is not anywhere in particular.
        let Some(t) = p.created_at.as_deref().and_then(parse_instant) else {
            continue;
        };
        earliest = Some(earliest.map_or(t, |e| e.min(t)));
        latest = Some(latest.map_or(t, |l| l.max(t)));
        by_key.entry(truncate(t, hourly)).or_default().push(p);
    }

    let (Some(observed_start), Some(observed_end)) = (earliest, latest) else {
        // Nothing datable in the selection. An axis with no instants on it is not a range
        // of empty buckets, it is no chart at all.
        return Vec::new();
    };
    // The requested range wins where it is known, so two charts drawn with the same
    // `window` share an axis even when one of them has no calls near its start. Where it
    // is not, the data draws its own bounds. Either way the observed calls stay inside.
    let start = floor.and_then(parse_instant).unwrap_or(observed_start);
    let end = until.and_then(parse_instant).unwrap_or(now);

    let step = if hourly {
        Duration::hours(1)
    } else {
        Duration::days(1)
    };
    let mut out = Vec::new();
    let mut at = truncate_dt(start.min(observed_start), hourly);
    let last = truncate_dt(end.max(observed_end), hourly);
    while at <= last {
        let key = stamp_bucket(at);
        let empty: Vec<&Point> = Vec::new();
        let group = by_key.get(&key).unwrap_or(&empty);
        out.push(Bucket {
            bucket: key,
            totals: totals(group.iter().copied()),
        });
        at += step;
    }
    out
}

fn parse_instant(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn truncate(t: DateTime<Utc>, hourly: bool) -> String {
    stamp_bucket(truncate_dt(t, hourly))
}

fn truncate_dt(t: DateTime<Utc>, hourly: bool) -> DateTime<Utc> {
    let hour = if hourly { t.hour() } else { 0 };
    Utc.with_ymd_and_hms(t.year(), t.month(), t.day(), hour, 0, 0)
        .single()
        .unwrap_or(t)
}

fn stamp_bucket(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `query_row` returns `Err(QueryReturnedNoRows)` for a missing row; the API wants that as
/// a 404, which is an `Ok(None)` here.
trait OptionalRow<T> {
    fn optional_row(self) -> Result<Option<T>>;
}

impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional_row(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
