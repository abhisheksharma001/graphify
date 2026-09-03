use graphify::ended_reason::group;

/// Every case is a real-shaped Vapi code. Ordering matters as much as matching, so
/// several entries deliberately satisfy a later rule too.
const CASES: &[(Option<&str>, &str)] = &[
    // timeout
    (Some("silence-timed-out"), "timeout"),
    (Some("exceeded-max-duration"), "timeout"),
    // start-error — wins over the `assistant-` and `-returning-` rules below
    (
        Some("call.start.error-vapifault-assistant-not-found"),
        "start-error",
    ),
    (Some("assistant-not-found"), "start-error"),
    (Some("assistant-request-returned-error"), "start-error"),
    (Some("scheduled-call-deleted"), "start-error"),
    // transfer-error
    (
        Some("call.in-progress.error-transfer-failed"),
        "transfer-error",
    ),
    (
        Some("call.in-progress.error-assistant-transfer-failed"),
        "transfer-error",
    ),
    // stt-error — beats the `pipeline` rule
    (
        Some("pipeline-error-deepgram-transcriber-failed"),
        "stt-error",
    ),
    (
        Some("call.in-progress.error-returning-invalid-payload"),
        "stt-error",
    ),
    // tts-error — all three beat the `pipeline` rule
    (
        Some("pipeline-error-eleven-labs-voice-not-found"),
        "tts-error",
    ),
    (
        Some("pipeline-error-eleven-labs-out-of-credits"),
        "tts-error",
    ),
    (Some("pipeline-error-cartesia-quota-exceeded"), "tts-error"),
    // llm-error
    (Some("pipeline-error-openai-llm-failed"), "llm-error"),
    (
        Some("call.in-progress.error-providerfault-openai-500-server-error"),
        "llm-error",
    ),
    (
        Some("call.in-progress.error-providerfault-anthropic-429-rate-limit"),
        "llm-error",
    ),
    // transport
    (
        Some("call.in-progress.error-sip-telephony-provider-failed-to-connect-call"),
        "transport",
    ),
    (Some("twilio-failed-to-connect-call"), "transport"),
    (Some("vonage-rejected"), "transport"),
    (Some("call.in-progress.error-vapifault-worker-died"), "transport"),
    (Some("websocket-connection-closed"), "transport"),
    // customer
    (Some("customer-ended-call"), "customer"),
    (Some("customer-busy"), "customer"),
    (Some("voicemail"), "customer"),
    // assistant
    (Some("assistant-ended-call"), "assistant"),
    (Some("assistant-said-end-call-phrase"), "assistant"),
    // unknown
    (None, "unknown"),
    (Some(""), "unknown"),
    (Some("   "), "unknown"),
    // other
    (Some("unknown-error"), "other"),
    (Some("db-error"), "other"),
];

#[test]
fn every_code_lands_in_the_expected_group() {
    for (code, want) in CASES {
        assert_eq!(group(*code), *want, "code was {code:?}");
    }
}

#[test]
fn all_eleven_groups_are_covered_by_the_cases() {
    let mut seen: Vec<&str> = CASES.iter().map(|(_, g)| *g).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen,
        [
            "assistant",
            "customer",
            "llm-error",
            "other",
            "start-error",
            "stt-error",
            "timeout",
            "transfer-error",
            "transport",
            "tts-error",
            "unknown",
        ]
    );
}

#[test]
fn matching_ignores_case() {
    assert_eq!(group(Some("Silence-Timed-Out")), "timeout");
}
