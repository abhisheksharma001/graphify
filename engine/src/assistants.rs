//! `graphify assistants`: the org's tools and assistants, slimmed on the way in.
//!
//! A live assistant is ~49 KB of JSON, most of it prompt text and plan boilerplate that no
//! chart reads. What lands is the handful of columns the dashboard draws, the system
//! prompt with its hash, and the structured-data schema — the schema being the contract
//! the brain (S-19) later extracts against.
//!
//! Tools are fetched first because `is_transfer` is what lets the extractor recognise a
//! transfer that `endedReason` alone would not name.

use crate::db::{Assistant, Db, Tool};
use crate::extract::{at, sha256_hex, str_at};
use crate::now;
use crate::vapi::{fetch_all_at, Retry};
use anyhow::{bail, Result};
use serde_json::Value;
use std::fmt;

pub struct Opts {
    pub org: String,
    pub base: String,
    pub key: String,
}

#[derive(Debug)]
pub struct Report {
    pub org: String,
    pub tools: usize,
    pub written: usize,
    pub unchanged: usize,
    /// Every assistant the org has, in the order Vapi listed them. An assistant with no
    /// name is listed by id: that is its real identity, not a stand-in for a missing one.
    pub names: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "org {}: {} tools, {} assistants written, {} unchanged",
            self.org, self.tools, self.written, self.unchanged
        )
    }
}

pub async fn run(db: &Db, opts: &Opts) -> Result<Report> {
    let Some(org) = db.org_by_name(&opts.org)? else {
        bail!("no org named {}", opts.org);
    };
    let fetched_at = now();
    let retry = Retry::default();

    let mut tools = 0;
    for raw in fetch_all_at(&opts.base, &opts.key, "tool", retry).await? {
        let Some(row) = tool_row(&raw, org.id, &fetched_at) else {
            continue;
        };
        db.upsert_tool(&row)?;
        tools += 1;
    }

    let mut raw = fetch_all_at(&opts.base, &opts.key, "assistant", retry).await?;
    raw.extend(squad_assistants(
        &fetch_all_at(&opts.base, &opts.key, "squad", retry).await?,
    ));

    let (mut written, mut unchanged) = (0, 0);
    let mut names = Vec::new();
    for one in &raw {
        let Some(row) = assistant_row(one, org.id, &fetched_at) else {
            continue;
        };
        names.push(row.name.clone().unwrap_or_else(|| row.id.clone()));
        // Vapi versions an assistant on every edit, and the prompt is the part that
        // changes what the brain sees, so those two together decide staleness.
        if db.assistant_fingerprint(&row.id)?
            == Some((row.version.clone(), row.prompt_sha256.clone()))
        {
            unchanged += 1;
            continue;
        }
        db.upsert_assistant(&row)?;
        written += 1;
    }

    Ok(Report {
        org: org.name,
        tools,
        written,
        unchanged,
        names,
    })
}

/// A tool without an id cannot be keyed on, and an assistant's `toolIds` could never
/// point at it, so it is dropped rather than stored under a made-up key.
fn tool_row(raw: &Value, org_id: i64, fetched_at: &str) -> Option<Tool> {
    let kind = str_at(raw, &["type"]);
    Some(Tool {
        id: str_at(raw, &["id"])?,
        org_id,
        name: str_at(raw, &["function", "name"]),
        is_transfer: kind.as_deref() == Some("transferCall"),
        kind,
        fetched_at: Some(fetched_at.to_string()),
    })
}

fn assistant_row(raw: &Value, org_id: i64, fetched_at: &str) -> Option<Assistant> {
    let system_prompt = system_prompt(raw);
    Some(Assistant {
        id: str_at(raw, &["id"])?,
        org_id,
        name: str_at(raw, &["name"]),
        version: str_at(raw, &["latestVersion"]),
        model_provider: str_at(raw, &["model", "provider"]),
        model: str_at(raw, &["model", "model"]),
        voice_provider: str_at(raw, &["voice", "provider"]),
        transcriber_provider: str_at(raw, &["transcriber", "provider"]),
        transcriber_model: str_at(raw, &["transcriber", "model"]),
        // An assistant with no system prompt has no hash either. Hashing the empty string
        // would give every prompt-less assistant the same fingerprint.
        prompt_sha256: system_prompt.as_deref().map(sha256_hex),
        system_prompt,
        first_message: str_at(raw, &["firstMessage"]),
        tool_ids: json_at(raw, &["model", "toolIds"]),
        structured_schema: structured_schema(raw),
        fetched_at: Some(fetched_at.to_string()),
    })
}

/// The first `system` message on the model. Vapi allows several, but only the first one
/// is the assistant's instructions; the rest are few-shot turns.
fn system_prompt(raw: &Value) -> Option<String> {
    at(raw, &["model", "messages"])?
        .as_array()?
        .iter()
        .find(|m| str_at(m, &["role"]).as_deref() == Some("system"))
        .and_then(|m| str_at(m, &["content"]))
}

/// The structured-data schema, or NULL when the plan is switched off. A disabled plan can
/// still carry a leftover schema, and storing it would promise columns nothing fills.
fn structured_schema(raw: &Value) -> Option<String> {
    let plan = at(raw, &["analysisPlan", "structuredDataPlan"])?;
    if plan.get("enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    json_at(plan, &["schema"])
}

/// Squad members can carry a whole assistant inline instead of referring to one by id.
/// Only the ones that came with an id are storable — an inline-only assistant has nothing
/// to key a row on, and no call ever names it either.
fn squad_assistants(squads: &[Value]) -> Vec<Value> {
    squads
        .iter()
        .filter_map(|s| at(s, &["members"])?.as_array())
        .flatten()
        .filter(|m| str_at(m, &["assistant", "id"]).is_some())
        .filter_map(|m| m.get("assistant").cloned())
        .collect()
}

fn json_at(v: &Value, path: &[&str]) -> Option<String> {
    Some(at(v, path)?.to_string())
}
