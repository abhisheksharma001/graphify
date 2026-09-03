use graphify::extract::{extract, sha256_hex};
use serde_json::{json, Value};
use std::collections::HashSet;

const FIXTURE: &str = include_str!("fixtures/call_ended_transfer.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

/// The one tool in `fixtures/tools.json` whose `type` is `transferCall`.
fn transfer_tools() -> HashSet<String> {
    HashSet::from(["acmeTransferAssistant_SpringfieldProd".to_string()])
}

fn none() -> HashSet<String> {
    HashSet::new()
}

/// Every number the spec's acceptance line names, from the real payload shape.
#[test]
fn the_fixture_extracts_the_numbers_the_spec_names() {
    let (c, tools) = extract(&fixture(), 7, &transfer_tools()).unwrap();

    assert_eq!(c.tool_calls, Some(1));
    assert_eq!(c.tool_failures, Some(0));
    assert_eq!(c.transferred, Some(true));
    assert_eq!(c.turns, Some(2));
    assert_eq!(c.lat_turn_avg_ms, Some(4553.5));
    assert_eq!(c.lat_turn_p95_ms, Some(6030.0));
    assert_eq!(c.cost_vapi, Some(0.0248));
    assert_eq!(c.success_eval.as_deref(), Some("true"));

    let structured: Value = serde_json::from_str(c.structured.as_ref().unwrap()).unwrap();
    assert_eq!(structured["call_intent"], "general_info");

    // The rest of the mapping, so a regression in one column is not hidden by the eight above.
    assert_eq!(c.org_id, 7);
    assert_eq!(c.assistant_version.as_deref(), Some("v7"));
    assert_eq!(c.call_type.as_deref(), Some("inboundPhoneCall"));
    assert_eq!(c.ended_reason.as_deref(), Some("assistant-forwarded-call"));
    // A forwarded call ended cleanly; `transfer-error` is for forwards that broke.
    assert_eq!(c.ended_group.as_deref(), Some("assistant"));
    assert_eq!(c.transfer_destination.as_deref(), Some("+15550000001"));
    assert_eq!(c.lat_turn_p50_ms, Some(3077.0));
    assert_eq!(c.llm_prompt_tokens, Some(30002));
    assert_eq!(c.tts_characters, Some(199));
    assert!((c.duration_s.unwrap() - 29.799).abs() < 1e-9);
    assert!((c.cost_analysis.unwrap() - 0.0413).abs() < 1e-9);
    assert_eq!(
        c.recording_url.as_deref(),
        Some("https://example.invalid/recordings/c01-mono.wav")
    );
    // The caller owns the clock; extract stays pure.
    assert_eq!(c.synced_at, None);

    assert_eq!(tools.len(), 1);
    let t = &tools[0];
    assert_eq!(t.name.as_deref(), Some("acmeTransferAssistant_SpringfieldProd"));
    assert_eq!(t.failed, Some(false));
    assert_eq!(t.seconds_from_start, Some(27.333));
    assert_eq!(t.arguments.as_deref(), Some(r#"{"destination": "+15550000001"}"#));
    assert_eq!(t.result_excerpt.as_deref(), Some("Transfer initiated."));
}

#[test]
fn slim_is_under_10_kb_and_holds_no_duplicates() {
    let (c, _) = extract(&fixture(), 1, &transfer_tools()).unwrap();
    let slim = c.slim.unwrap();
    assert!(slim.len() < 10_240, "slim was {} bytes", slim.len());

    let s: Value = serde_json::from_str(&slim).unwrap();
    for gone in ["messages", "monitor", "transport"] {
        assert!(s.get(gone).is_none(), "slim still carries {gone}");
    }
    for gone in ["messagesOpenAIFormatted", "variables", "variableValues", "logUrl"] {
        assert!(s["artifact"].get(gone).is_none(), "slim still carries artifact.{gone}");
    }
    // What the call drawer draws has to survive.
    for kept in ["costs", "analysis", "destination", "transcript"] {
        assert!(s.get(kept).is_some(), "slim dropped {kept}");
    }
    for kept in ["performanceMetrics", "assistantActivations", "recordingUrl", "messages"] {
        assert!(s["artifact"].get(kept).is_some(), "slim dropped artifact.{kept}");
    }
}

#[test]
fn the_system_prompt_is_replaced_by_its_hash() {
    let (c, _) = extract(&fixture(), 1, &transfer_tools()).unwrap();
    let s: Value = serde_json::from_str(&c.slim.unwrap()).unwrap();
    let sys = s["artifact"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "system")
        .unwrap();

    assert_eq!(
        sys["prompt_sha256"],
        "a563b9cc4852995907d8a1e09c9fe9752ebb7c3e36c4a322fddcb44806505fab"
    );
    assert!(sys.get("message").is_none(), "the prompt text is still in slim");
    assert_eq!(sha256_hex(""), sha256_hex(""));
    assert_ne!(sha256_hex("a"), sha256_hex("b"));
}

/// The rule the whole extractor exists to keep: absent is not zero.
#[test]
fn everything_missing_stays_null() {
    let (c, tools) = extract(&json!({ "id": "c1" }), 1, &none()).unwrap();

    assert_eq!(c.id, "c1");
    assert_eq!(c.cost, None);
    assert_eq!(c.cost_vapi, None);
    assert_eq!(c.cost_analysis, None);
    assert_eq!(c.duration_s, None);
    assert_eq!(c.turns, None);
    assert_eq!(c.tool_calls, None);
    assert_eq!(c.tool_failures, None);
    assert_eq!(c.transferred, None);
    assert_eq!(c.lat_turn_avg_ms, None);
    assert_eq!(c.lat_turn_p50_ms, None);
    assert_eq!(c.lat_turn_p95_ms, None);
    assert_eq!(c.turn_latencies, None);
    assert_eq!(c.ended_group, None);
    assert_eq!(c.recording_url, None);
    assert!(tools.is_empty());
}

/// An in-progress call has an artifact, just an empty one. Counts are 0, not NULL, because
/// the arrays are really there and really empty.
#[test]
fn an_empty_artifact_counts_zero_but_a_missing_one_stays_null() {
    let empty = json!({
        "id": "c1",
        "artifact": { "messages": [], "performanceMetrics": { "turnLatencies": [] } }
    });
    let (c, _) = extract(&empty, 1, &none()).unwrap();
    assert_eq!(c.tool_calls, Some(0));
    assert_eq!(c.tool_failures, Some(0));
    assert_eq!(c.turns, Some(0));
    // Still no latency samples, so the percentiles have nothing to report.
    assert_eq!(c.lat_turn_p95_ms, None);
}

#[test]
fn a_transfer_tool_counts_even_when_the_ended_reason_does_not() {
    let raw = json!({
        "id": "c1",
        "endedReason": "customer-ended-call",
        "artifact": { "messages": [{
            "role": "tool_calls",
            "toolCalls": [{ "id": "t1", "function": { "name": "acmeTransferAssistant_SpringfieldProd" } }]
        }]}
    });
    assert_eq!(extract(&raw, 1, &transfer_tools()).unwrap().0.transferred, Some(true));
    // The same call, if that tool were not a transfer tool for this org.
    assert_eq!(extract(&raw, 1, &none()).unwrap().0.transferred, Some(false));
}

#[test]
fn a_destination_number_alone_is_a_transfer() {
    let raw = json!({ "id": "c1", "destination": { "type": "number", "number": "+15550000001" } });
    assert_eq!(extract(&raw, 1, &none()).unwrap().0.transferred, Some(true));
}

/// No evidence means "no" for a finished call and "not yet" for a running one.
#[test]
fn no_evidence_is_false_once_ended_and_null_while_running() {
    let ended = json!({ "id": "c1", "status": "ended", "endedReason": "customer-ended-call" });
    assert_eq!(extract(&ended, 1, &none()).unwrap().0.transferred, Some(false));

    let running = json!({ "id": "c1", "status": "in-progress" });
    assert_eq!(extract(&running, 1, &none()).unwrap().0.transferred, None);
}

#[test]
fn an_empty_or_complaining_result_is_a_failure() {
    let raw = json!({ "id": "c1", "artifact": { "messages": [
        { "role": "tool_calls", "toolCalls": [
            { "id": "t1", "function": { "name": "a" } },
            { "id": "t2", "function": { "name": "b" } },
            { "id": "t3", "function": { "name": "c" } },
            { "id": "t4", "function": { "name": "d" } }
        ]},
        { "role": "tool_call_result", "toolCallId": "t1", "result": "Booked." },
        { "role": "tool_call_result", "toolCallId": "t2", "result": "  " },
        { "role": "tool_call_result", "toolCallId": "t3", "result": "Upstream ERROR 500" },
        { "role": "tool_call_result", "toolCallId": "t4", "result": "Lookup Failed" }
    ]}});
    let (c, tools) = extract(&raw, 1, &none()).unwrap();

    assert_eq!(c.tool_calls, Some(4));
    assert_eq!(c.tool_failures, Some(3));
    let flags: Vec<_> = tools.iter().map(|t| t.failed).collect();
    assert_eq!(flags, vec![Some(false), Some(true), Some(true), Some(true)]);
}

/// A tool that never came back is unknown, not failed.
#[test]
fn a_tool_call_with_no_result_is_null_not_failed() {
    let raw = json!({ "id": "c1", "artifact": { "messages": [
        { "role": "tool_calls", "toolCalls": [{ "id": "t1", "function": { "name": "a" } }] }
    ]}});
    let (c, tools) = extract(&raw, 1, &none()).unwrap();
    assert_eq!(c.tool_calls, Some(1));
    assert_eq!(c.tool_failures, Some(0));
    assert_eq!(tools[0].failed, None);
    assert_eq!(tools[0].result_excerpt, None);
}

#[test]
fn duration_needs_both_ends() {
    let both = json!({
        "id": "c1",
        "startedAt": "2026-09-03T17:24:27.607Z",
        "endedAt": "2026-09-03T17:24:57.406Z"
    });
    assert!((extract(&both, 1, &none()).unwrap().0.duration_s.unwrap() - 29.799).abs() < 1e-9);

    let open = json!({ "id": "c1", "startedAt": "2026-09-03T17:24:27.607Z" });
    assert_eq!(extract(&open, 1, &none()).unwrap().0.duration_s, None);

    let junk = json!({ "id": "c1", "startedAt": "yesterday", "endedAt": "today" });
    assert_eq!(extract(&junk, 1, &none()).unwrap().0.duration_s, None);
}

#[test]
fn a_call_without_an_id_is_an_error() {
    let err = extract(&json!({ "status": "ended" }), 1, &none()).unwrap_err();
    assert!(err.to_string().contains("no id"), "error was: {err}");
}
