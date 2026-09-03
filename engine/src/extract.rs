//! Raw Vapi call JSON -> one `calls` row, its `tool_calls` rows, and the slim JSON kept
//! for the call drawer. Raw never lands: `slim` is the only blob written.
//!
//! Every Vapi field is optional in practice — an in-progress call has an empty artifact,
//! and analysis fields only appear once the call ends. A field that is not there stays
//! NULL. Nothing here ever substitutes 0 for "we don't know".

use crate::db::{Call, ToolCall};
use crate::ended_reason;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Longest `result` kept on a `tool_calls` row. The full text still lives in `slim`.
const RESULT_EXCERPT: usize = 500;

/// Turn one raw call into the rows the sync path writes.
///
/// `transfer_tools` holds the names of tools whose `tools.is_transfer` is 1. A call that
/// invoked one counts as transferred even when `endedReason` says something else, which
/// happens whenever the transfer itself is what ended the call badly.
///
/// `synced_at` is deliberately left NULL: the caller owns the clock, so this stays a pure
/// function and its tests need no time control.
pub fn extract(
    raw: &Value,
    org_id: i64,
    transfer_tools: &HashSet<String>,
) -> Result<(Call, Vec<ToolCall>)> {
    let Some(id) = str_at(raw, &["id"]) else {
        bail!("call has no id");
    };

    let started_at = str_at(raw, &["startedAt"]);
    let ended_at = str_at(raw, &["endedAt"]);
    let ended_reason = str_at(raw, &["endedReason"]);
    let messages = at(raw, &["artifact", "messages"]).and_then(Value::as_array);
    let tools = tool_rows(messages);
    let transferred = transferred(raw, ended_reason.as_deref(), &tools, transfer_tools);

    // Latencies. `turns` counts the entries Vapi recorded, whether or not each one
    // carries a usable `turnLatency`, so it stays a turn count and not a sample count.
    let turn_latencies = at(raw, &["artifact", "performanceMetrics", "turnLatencies"])
        .and_then(Value::as_array);
    let mut turn_ms: Vec<f64> = turn_latencies
        .map(|a| a.iter().filter_map(|t| f64_at(t, &["turnLatency"])).collect())
        .unwrap_or_default();
    turn_ms.sort_by(f64::total_cmp);

    let call = Call {
        id,
        org_id,
        assistant_id: str_at(raw, &["assistantId"]),
        assistant_version: str_at(
            raw,
            &["artifact", "assistantActivations", "0", "assistantVersion"],
        ),
        phone_number_id: str_at(raw, &["phoneNumberId"]),
        call_type: str_at(raw, &["type"]),
        status: str_at(raw, &["status"]),
        created_at: str_at(raw, &["createdAt"]),
        duration_s: seconds_between(started_at.as_deref(), ended_at.as_deref()),
        ended_group: ended_reason
            .as_deref()
            .map(|r| ended_reason::group(Some(r)).to_string()),
        started_at,
        ended_reason,
        ended_at,

        cost: f64_at(raw, &["cost"]),
        cost_stt: f64_at(raw, &["costBreakdown", "stt"]),
        cost_llm: f64_at(raw, &["costBreakdown", "llm"]),
        cost_tts: f64_at(raw, &["costBreakdown", "tts"]),
        cost_vapi: f64_at(raw, &["costBreakdown", "vapi"]),
        cost_transport: f64_at(raw, &["costBreakdown", "transport"]),
        cost_analysis: analysis_cost(raw),
        llm_prompt_tokens: i64_at(raw, &["costBreakdown", "llmPromptTokens"]),
        llm_completion_tokens: i64_at(raw, &["costBreakdown", "llmCompletionTokens"]),
        llm_cached_tokens: i64_at(raw, &["costBreakdown", "llmCachedPromptTokens"]),
        tts_characters: i64_at(raw, &["costBreakdown", "ttsCharacters"]),

        transferred,
        transfer_destination: str_at(raw, &["destination", "number"]),
        tool_calls: messages.map(|_| tools.len() as i64),
        tool_failures: messages.map(|m| tool_failures(m)),
        turns: turn_latencies.map(|a| a.len() as i64),

        lat_turn_avg_ms: f64_at(raw, &["artifact", "performanceMetrics", "turnLatencyAverage"]),
        lat_turn_p50_ms: percentile(&turn_ms, 0.50),
        lat_turn_p95_ms: percentile(&turn_ms, 0.95),
        lat_model_avg_ms: f64_at(raw, &["artifact", "performanceMetrics", "modelLatencyAverage"]),
        lat_voice_avg_ms: f64_at(raw, &["artifact", "performanceMetrics", "voiceLatencyAverage"]),
        lat_transcriber_avg_ms: f64_at(
            raw,
            &["artifact", "performanceMetrics", "transcriberLatencyAverage"],
        ),
        lat_endpointing_avg_ms: f64_at(
            raw,
            &["artifact", "performanceMetrics", "endpointingLatencyAverage"],
        ),
        turn_latencies: turn_latencies.map(|a| Value::Array(a.clone()).to_string()),

        success_eval: str_at(raw, &["analysis", "successEvaluation"]),
        summary: str_at(raw, &["analysis", "summary"]),
        structured: at(raw, &["analysis", "structuredData"]).map(Value::to_string),
        transcript: str_at(raw, &["transcript"]),
        recording_url: str_at(raw, &["artifact", "recordingUrl"]),
        slim: Some(slim(raw).to_string()),
        synced_at: None,
    };

    Ok((call, tools))
}

/// Every `toolCalls[]` entry across every `tool_calls` message, paired with its result.
fn tool_rows(messages: Option<&Vec<Value>>) -> Vec<ToolCall> {
    let Some(messages) = messages else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for m in messages {
        if str_at(m, &["role"]).as_deref() != Some("tool_calls") {
            continue;
        }
        let seconds_from_start = f64_at(m, &["secondsFromStart"]);
        for c in at(m, &["toolCalls"]).and_then(Value::as_array).into_iter().flatten() {
            let id = str_at(c, &["id"]);
            let result = id.as_deref().and_then(|id| result_for(messages, id));
            out.push(ToolCall {
                name: str_at(c, &["function", "name"]),
                seconds_from_start,
                // No result message at all means the call never came back, which is not
                // the same as a result that reported an error. Leave it NULL.
                failed: result.as_deref().map(is_failure),
                arguments: str_at(c, &["function", "arguments"]),
                result_excerpt: result.map(|r| excerpt(&r)),
            });
        }
    }
    out
}

fn result_for(messages: &[Value], tool_call_id: &str) -> Option<String> {
    messages.iter().find_map(|m| {
        (str_at(m, &["role"]).as_deref() == Some("tool_call_result")
            && str_at(m, &["toolCallId"]).as_deref() == Some(tool_call_id))
        .then(|| str_at(m, &["result"]).unwrap_or_default())
    })
}

/// Count failures over the result messages themselves, so a result that arrives without a
/// matching invocation still shows up in the total.
fn tool_failures(messages: &[Value]) -> i64 {
    messages
        .iter()
        .filter(|m| str_at(m, &["role"]).as_deref() == Some("tool_call_result"))
        .filter(|m| is_failure(&str_at(m, &["result"]).unwrap_or_default()))
        .count() as i64
}

/// Vapi has no failure flag on a tool result — the tool's own text is all there is.
fn is_failure(result: &str) -> bool {
    let r = result.trim().to_ascii_lowercase();
    r.is_empty() || r.contains("error") || r.contains("failed")
}

fn excerpt(s: &str) -> String {
    match s.char_indices().nth(RESULT_EXCERPT) {
        Some((cut, _)) => s[..cut].to_string(),
        None => s.to_string(),
    }
}

/// Three ways a call can be a transfer; any one of them is enough.
///
/// With no evidence the answer is `false` only for a call that actually ended — a call
/// still running has not transferred *yet*, which is a NULL, not a no.
fn transferred(
    raw: &Value,
    ended_reason: Option<&str>,
    tools: &[ToolCall],
    transfer_tools: &HashSet<String>,
) -> Option<bool> {
    let by_reason = ended_reason == Some("assistant-forwarded-call");
    let by_destination = at(raw, &["destination", "number"]).is_some_and(Value::is_string);
    let by_tool = tools
        .iter()
        .filter_map(|t| t.name.as_deref())
        .any(|n| transfer_tools.contains(n));

    if by_reason || by_destination || by_tool {
        Some(true)
    } else {
        ended_reason.map(|_| false)
    }
}

/// The three analysis line items, summed. Vapi bills them separately and omits the block
/// entirely when no analysis ran, which is a NULL cost rather than a free one.
fn analysis_cost(raw: &Value) -> Option<f64> {
    let b = at(raw, &["costBreakdown", "analysisCostBreakdown"])?;
    Some(
        ["summary", "structuredData", "successEvaluation"]
            .iter()
            .filter_map(|k| f64_at(b, &[k]))
            .sum(),
    )
}

/// Nearest-rank percentile: the p95 of two turns is the slower turn, not a blend of both.
fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank - 1).copied()
}

/// Seconds between two ISO-8601 instants. NULL if either is missing or unparseable.
fn seconds_between(from: Option<&str>, to: Option<&str>) -> Option<f64> {
    let from = chrono::DateTime::parse_from_rfc3339(from?).ok()?;
    let to = chrono::DateTime::parse_from_rfc3339(to?).ok()?;
    Some((to - from).num_milliseconds() as f64 / 1000.0)
}

/// The D-12 slim: drop what is duplicated, ephemeral, or a credential, keep what the call
/// drawer draws. Written as removals so a new Vapi field shows up in the drawer by
/// default instead of being silently dropped.
fn slim(raw: &Value) -> Value {
    let mut out = raw.clone();
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    // `messages` duplicates `artifact.messages`; `assistant`/`squad` are stored once per
    // version; `monitor` holds live listen/control URLs that are dead by sync time.
    for k in ["messages", "monitor", "transport", "assistant", "squad"] {
        o.remove(k);
    }
    if let Some(a) = o.get_mut("artifact").and_then(Value::as_object_mut) {
        // `messagesOpenAIFormatted` is the same transcript again; `variables` and
        // `variableValues` carry caller PII; `logUrl` is a presigned URL.
        for k in ["messagesOpenAIFormatted", "variables", "variableValues", "logUrl"] {
            a.remove(k);
        }
        for m in a.get_mut("messages").and_then(Value::as_array_mut).into_iter().flatten() {
            if str_at(m, &["role"]).as_deref() == Some("system") {
                let sha = sha256_hex(&str_at(m, &["message"]).unwrap_or_default());
                *m = json!({ "role": "system", "prompt_sha256": sha });
            }
        }
    }
    out
}

/// Hex SHA-256, so a call can point at a system prompt without carrying a copy of it.
pub fn sha256_hex(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

/// Walk a JSON path. A numeric segment indexes an array, so `["a", "0", "b"]` works.
pub fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for key in path {
        cur = match cur {
            Value::Array(a) => a.get(key.parse::<usize>().ok()?)?,
            _ => cur.get(key)?,
        };
    }
    Some(cur)
}

pub fn str_at(v: &Value, path: &[&str]) -> Option<String> {
    at(v, path)?.as_str().map(str::to_string)
}

fn f64_at(v: &Value, path: &[&str]) -> Option<f64> {
    at(v, path)?.as_f64()
}

fn i64_at(v: &Value, path: &[&str]) -> Option<i64> {
    at(v, path)?.as_i64()
}
