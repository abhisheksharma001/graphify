//! The decision connector, against a mock server: what goes out, what comes back, and what
//! never leaves at all.
//!
//! `engine/tests/outbound.rs` reads this adapter's *text* and proves it can only ever spell
//! a POST. That guard cannot see what a request actually was, only what the source says, so
//! the last test here watches the wire the way the data connector's has since S-49. The
//! rest are about the boundary in the other direction: a provider that answers half the
//! questions, or answers one in the wrong shape, or does not say what the call cost, must
//! fail by name here rather than become a confident number three screens later.
//!
//! No test in this file reaches the network, and the key below is not a key.

use graphify::decide::{Answer, DecisionModel, Question, Questions};
use graphify::jev::{Jev, Usage};
use serde_json::{json, Value};
use std::time::Duration;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Not a key. It is here so that an error can be searched for it.
const KEY: &str = "ts-key-0000-not-real-4f3a";

/// The question the first real caller will ask, in miniature.
fn wants_human() -> Questions {
    Questions::new(vec![(
        "wants_human",
        Question::Binary {
            instructions: "The caller asked to be connected to a human being.".into(),
            when_true: "The caller asked for a person.".into(),
            when_false: "The caller never asked for a person.".into(),
        },
    )])
    .expect("the question set is askable")
}

/// A server that answers everything with `reply`, and an adapter pointed at it.
async fn answering(reply: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(reply))
        .mount(&server)
        .await;
    server
}

fn jev(server: &MockServer) -> Jev {
    Jev::at(&server.uri(), KEY, Duration::from_secs(5))
}

/// The one request that left, parsed.
async fn sent_body(server: &MockServer) -> Value {
    let sent = server.received_requests().await.expect("the mock recorded requests");
    assert_eq!(sent.len(), 1, "expected exactly one request, got {}", sent.len());
    serde_json::from_slice(&sent[0].body).expect("the request body is JSON")
}

#[tokio::test]
async fn one_binary_question_makes_one_post_in_the_vendors_shape() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "wants_human": { "noul": 0.97 } },
        "usage": { "input_tokens": 448 },
    }))
    .await;

    let asked = wants_human();
    let (answers, usage) = jev(&server)
        .ask("caller: put me through to a person", &asked)
        .await
        .expect("the provider answered");

    assert_eq!(answers.model(), "jev-1.13.0");
    assert_eq!(answers.get("wants_human"), Some(&Answer::Binary(0.97)));
    assert_eq!(
        usage,
        Usage {
            input_tokens: 448,
            usd: 448.0 * 0.042 / 1_000_000.0,
        }
    );

    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1, "one question set is one call");
    assert_eq!(sent[0].method.as_str(), "POST");
    assert_eq!(sent[0].url.path(), "/v1/systemone");
    assert_eq!(
        sent[0].headers.get("authorization").expect("the key was sent"),
        &format!("Bearer {KEY}")
    );

    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(body["model"], "jev-1.13.0");
    assert_eq!(body["state"], "caller: put me through to a person");
    assert_eq!(body["questions"]["wants_human"]["type"], "noul");
    assert_eq!(
        body["questions"]["wants_human"]["criteria"]["true"],
        "The caller asked for a person."
    );
    assert_eq!(
        body["questions"]["wants_human"]["criteria"]["false"],
        "The caller never asked for a person."
    );
}

/// The pin is what we ask for; the reply is what we believe. A version that moved
/// underneath us has to be visible at the boundary, not at the next recalibration.
#[tokio::test]
async fn the_model_carried_back_is_the_one_that_answered_not_the_one_we_asked_for() {
    let server = answering(json!({
        "model": "jev-1.14.0",
        "answers": { "wants_human": { "noul": 0.11 } },
        "usage": { "input_tokens": 100 },
    }))
    .await;

    let asked = wants_human();
    let (answers, _) = jev(&server).ask("a call", &asked).await.expect("answered");

    assert_eq!(answers.model(), "jev-1.14.0", "a pin that moved must show here");
    assert_eq!(
        sent_body(&server).await["model"],
        "jev-1.13.0",
        "and we must still have asked for the pin"
    );
}

#[tokio::test]
async fn a_choice_goes_out_as_an_object_and_a_score_as_an_ordered_list() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": {
            "tone": { "choice": "angry", "confidence": 0.88 },
            "urgency": { "score": 1.5 },
        },
        "usage": { "input_tokens": 61 },
    }))
    .await;

    let asked = Questions::new(vec![
        (
            "tone",
            Question::Choice {
                instructions: "The caller's tone.".into(),
                options: vec![
                    ("angry".into(), "hostile".into()),
                    ("calm".into(), "neutral or polite".into()),
                ],
            },
        ),
        (
            "urgency",
            Question::Score {
                instructions: "How soon does this need a person?".into(),
                levels: vec!["can wait".into(), "this week".into(), "today".into()],
            },
        ),
    ])
    .expect("the question set is askable");

    let (answers, _) = jev(&server).ask("a call", &asked).await.expect("answered");
    assert_eq!(
        answers.get("tone"),
        Some(&Answer::Choice {
            option: "angry".into(),
            confidence: 0.88,
        })
    );
    assert_eq!(answers.get("urgency"), Some(&Answer::Score(1.5)));

    let body = sent_body(&server).await;
    assert_eq!(body["questions"]["tone"]["type"], "choice");
    assert_eq!(body["questions"]["tone"]["criteria"]["angry"], "hostile");
    assert_eq!(body["questions"]["urgency"]["type"], "score");
    assert_eq!(
        body["questions"]["urgency"]["criteria"],
        json!(["can wait", "this week", "today"]),
        "a score's levels are ordered, so they are a list and not an object"
    );
}

/// Two questions about one state are one call: the state is billed once and the question
/// count does not move the latency.
#[tokio::test]
async fn every_question_about_one_state_goes_in_one_call() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "a": { "noul": 0.2 }, "b": { "noul": 0.8 } },
        "usage": { "input_tokens": 70 },
    }))
    .await;

    let one = |text: &str| Question::Binary {
        instructions: text.to_string(),
        when_true: "yes".into(),
        when_false: "no".into(),
    };
    let asked = Questions::new(vec![("a", one("First.")), ("b", one("Second."))]).unwrap();

    jev(&server).ask("a call", &asked).await.expect("answered");
    let body = sent_body(&server).await;
    assert_eq!(body["questions"].as_object().unwrap().len(), 2);
}

#[tokio::test]
async fn a_missing_probability_is_an_error_and_never_a_zero() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "wants_human": {} },
        "usage": { "input_tokens": 12 },
    }))
    .await;

    let asked = wants_human();
    let err = jev(&server).ask("a call", &asked).await.unwrap_err().to_string();
    assert!(err.contains("wants_human"), "{err}");
    assert!(err.contains("noul"), "{err}");
}

#[tokio::test]
async fn a_question_left_unanswered_is_an_error() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": {},
        "usage": { "input_tokens": 12 },
    }))
    .await;

    let asked = wants_human();
    let err = jev(&server).ask("a call", &asked).await.unwrap_err().to_string();
    assert!(err.contains("wants_human"), "{err}");
}

#[tokio::test]
async fn an_answer_to_a_question_nobody_asked_is_refused() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "wants_human": { "noul": 0.5 }, "wants_refund": { "noul": 0.9 } },
        "usage": { "input_tokens": 12 },
    }))
    .await;

    let asked = wants_human();
    let err = jev(&server).ask("a call", &asked).await.unwrap_err().to_string();
    assert!(err.contains("wants_refund"), "{err}");
}

#[tokio::test]
async fn an_answer_in_the_wrong_shape_is_refused_by_name() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "tone": { "noul": 0.9 } },
        "usage": { "input_tokens": 12 },
    }))
    .await;

    let asked = Questions::new(vec![(
        "tone",
        Question::Choice {
            instructions: "The caller's tone.".into(),
            options: vec![
                ("angry".into(), "hostile".into()),
                ("calm".into(), "polite".into()),
            ],
        },
    )])
    .unwrap();

    let err = jev(&server).ask("a call", &asked).await.unwrap_err().to_string();
    assert!(err.contains("tone"), "{err}");
}

#[tokio::test]
async fn a_probability_outside_its_range_is_refused() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "wants_human": { "noul": 1.4 } },
        "usage": { "input_tokens": 12 },
    }))
    .await;

    let asked = wants_human();
    let err = jev(&server).ask("a call", &asked).await.unwrap_err().to_string();
    assert!(err.contains("wants_human"), "{err}");
    assert!(err.contains("probability"), "{err}");
}

/// A reply with no usage is not a free call; it is a call whose price nobody knows. Booking
/// that as nothing is how a daily cap quietly stops working.
#[tokio::test]
async fn a_reply_that_does_not_say_what_it_cost_is_an_error() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "wants_human": { "noul": 0.5 } },
    }))
    .await;

    let asked = wants_human();
    let err = jev(&server).ask("a call", &asked).await.unwrap_err().to_string();
    assert!(err.contains("cost"), "{err}");
}

#[tokio::test]
async fn a_state_over_the_cap_never_leaves_the_process() {
    let server = answering(json!({})).await;
    let asked = wants_human();

    let err = jev(&server)
        .ask(&"x".repeat(24_001), &asked)
        .await
        .unwrap_err()
        .to_string();

    // The order matters: the claim this test exists for is that nothing left, so it is
    // asserted first. With it second, removing the cap reds on the message instead and the
    // failure says nothing about a request having left.
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the state was over the cap and a request left anyway"
    );
    assert!(err.contains("24000"), "{err}");
}

#[tokio::test]
async fn a_request_over_the_cap_never_leaves_the_process() {
    let server = answering(json!({})).await;
    let asked = Questions::new(vec![(
        "wants_human",
        Question::Binary {
            instructions: "The caller asked for a person.".into(),
            when_true: "t".repeat(30_000),
            when_false: "no".into(),
        },
    )])
    .unwrap();

    let err = jev(&server)
        .ask(&"x".repeat(20_000), &asked)
        .await
        .unwrap_err()
        .to_string();

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the request was over the cap and left anyway"
    );
    assert!(err.contains("48000"), "{err}");
}

#[tokio::test]
async fn a_provider_failure_is_an_error_that_does_not_carry_the_key() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream on fire"))
        .mount(&server)
        .await;

    let asked = wants_human();
    let err = format!(
        "{:#}",
        jev(&server).ask("a call", &asked).await.unwrap_err()
    );
    assert!(err.contains("500"), "{err}");
    assert!(err.contains("upstream on fire"), "{err}");
    assert!(!err.contains(KEY), "the key is in the error: {err}");
}

#[tokio::test]
async fn a_reply_that_is_not_json_is_an_error_and_not_an_empty_answer() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>nope</html>"))
        .mount(&server)
        .await;

    let asked = wants_human();
    let err = jev(&server).ask("a call", &asked).await.unwrap_err().to_string();
    assert!(err.contains("not JSON"), "{err}");
}

/// The seam's own view: run through `dyn DecisionModel`, which is the way every caller
/// after this step will see it, and which is also the proof the trait is object-safe.
#[tokio::test]
async fn the_same_answers_come_back_through_the_seam() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "wants_human": { "noul": 0.97 } },
        "usage": { "input_tokens": 448 },
    }))
    .await;

    let jev = jev(&server);
    let model: &dyn DecisionModel = &jev;
    assert_eq!(model.name(), "jev-1.13.0", "a ledger writes this, never the key");

    let answers = model.decide("a call", &wants_human()).await.expect("answered");
    assert_eq!(answers.get("wants_human"), Some(&Answer::Binary(0.97)));
}

/// The wire half of the first **Must never**, for the connector that is allowed to POST.
/// `engine/tests/outbound.rs` can only read this adapter's text; this watches what a run of
/// it actually sends, which is the guard S-72 could not write because nothing made a
/// request yet.
#[tokio::test]
async fn nothing_but_a_post_to_one_path_ever_leaves_for_the_decision_provider() {
    let server = answering(json!({
        "model": "jev-1.13.0",
        "answers": { "wants_human": { "noul": 0.5 } },
        "usage": { "input_tokens": 9 },
    }))
    .await;

    let jev = jev(&server);
    let asked = wants_human();
    jev.ask("first call", &asked).await.expect("answered");
    jev.ask("second call", &asked).await.expect("answered");

    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 2, "two asks, two calls");
    for req in &sent {
        assert_eq!(
            req.method.as_str(),
            "POST",
            "a {} left for the decision provider",
            req.method
        );
        assert_eq!(req.url.path(), "/v1/systemone");
    }
}
