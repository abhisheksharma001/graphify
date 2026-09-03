//! Vapi's `endedReason` is a long open-ended list of kebab-case codes. Charts need a
//! handful of buckets instead, so every code collapses into one of eleven groups.
//! Source: https://docs.vapi.ai/calls/call-ended-reason

/// Bucket an `endedReason`. Rules are ordered: the first match wins, so a code that
/// mentions both a transfer and a pipeline is a transfer error. One exception to plain
/// ordering: `voicemail` contains "voice", so the TTS rule skips it and it reaches the
/// customer rule where it belongs.
pub fn group(code: Option<&str>) -> &'static str {
    let code = match code {
        Some(c) => c.trim().to_ascii_lowercase(),
        None => return "unknown",
    };
    if code.is_empty() {
        return "unknown";
    }
    let c = code.as_str();

    if c.contains("silence-timed-out") || c.contains("exceeded-max-duration") {
        "timeout"
    } else if c.starts_with("call.start.error")
        || c.starts_with("assistant-not-")
        || c.starts_with("assistant-request-")
        || c == "scheduled-call-deleted"
    {
        "start-error"
    } else if c.contains("transfer") {
        "transfer-error"
    } else if c.contains("transcriber") || c.contains("-returning-") {
        "stt-error"
    } else if (c.contains("voice") && !c.starts_with("voicemail"))
        || c.contains("out-of-credits")
        || c.contains("quota")
    {
        "tts-error"
    } else if c.contains("llm") || c.contains("pipeline") || has_http_status(c) {
        "llm-error"
    } else if ["sip", "twilio", "vonage", "transport", "worker", "websocket"]
        .iter()
        .any(|k| c.contains(k))
    {
        "transport"
    } else if c.starts_with("customer-") || c.starts_with("voicemail") {
        "customer"
    } else if c.starts_with("assistant-") {
        "assistant"
    } else {
        "other"
    }
}

/// True if the code carries an embedded 4xx/5xx status, e.g. `...-openai-500-server-error`.
fn has_http_status(code: &str) -> bool {
    code.as_bytes().windows(5).any(|w| {
        w[0] == b'-'
            && (w[1] == b'4' || w[1] == b'5')
            && w[2].is_ascii_digit()
            && w[3].is_ascii_digit()
            && w[4] == b'-'
    })
}
