use graphify::vapi::{fetch_calls_at, FetchOpts, Retry};
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Retries with no real waiting, so the exhaustion test costs milliseconds.
const FAST: Retry = Retry { max: 5, base_ms: 0 };

/// A page of `n` calls with `createdAt` counting down from `top`, newest first. The
/// counter is zero-padded so string ordering and time ordering agree, which is what the
/// `createdAtLt` cursor and the `since` cutoff both rely on.
fn page(top: u32, n: u32) -> Value {
    let calls: Vec<Value> = (0..n)
        .map(|i| {
            let seq = top - i;
            json!({ "id": format!("call-{seq:04}"), "createdAt": at(seq) })
        })
        .collect();
    json!(calls)
}

fn at(seq: u32) -> String {
    format!("2026-09-03T00:00:00.{seq:04}Z")
}

fn opts(last: usize) -> FetchOpts {
    FetchOpts {
        last,
        ..FetchOpts::default()
    }
}

/// The spec's acceptance case: pages of 100 then 30, asking for 250.
#[tokio::test]
async fn a_short_page_ends_the_walk() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/call"))
        .and(query_param("limit", "100"))
        .and(query_param_is_missing("createdAtLt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(1000, 100)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/call"))
        .and(query_param("createdAtLt", at(901)))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(900, 30)))
        .mount(&server)
        .await;

    let calls = fetch_calls_at(&server.uri(), "k", &opts(250), FAST)
        .await
        .unwrap();

    assert_eq!(calls.len(), 130);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert_eq!(calls[0]["id"], "call-1000", "newest must stay first");
    assert_eq!(calls[129]["id"], "call-0871");
}

#[tokio::test]
async fn last_caps_the_page_size_and_the_result() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(1000, 10)))
        .mount(&server)
        .await;

    let calls = fetch_calls_at(&server.uri(), "k", &opts(10), FAST)
        .await
        .unwrap();

    assert_eq!(calls.len(), 10);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn since_cuts_the_page_off_and_stops() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(1000, 100)))
        .mount(&server)
        .await;

    let o = FetchOpts {
        last: 250,
        since: Some(at(996)),
        ..FetchOpts::default()
    };
    let calls = fetch_calls_at(&server.uri(), "k", &o, FAST).await.unwrap();

    // 1000..997 are newer than the cutoff; 996 is equal to it, so it and everything
    // older is dropped and no second page is asked for.
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[3]["id"], "call-0997");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn until_and_assistant_id_become_query_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("createdAtLt", at(500)))
        .and(query_param("assistantId", "asst-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(499, 3)))
        .mount(&server)
        .await;

    let o = FetchOpts {
        last: 250,
        until: Some(at(500)),
        assistant_id: Some("asst-1".to_string()),
        ..FetchOpts::default()
    };
    let calls = fetch_calls_at(&server.uri(), "k", &o, FAST).await.unwrap();

    assert_eq!(calls.len(), 3);
}

#[tokio::test]
async fn a_500_is_retried_and_the_next_try_counts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(1000, 5)))
        .mount(&server)
        .await;

    let calls = fetch_calls_at(&server.uri(), "k", &opts(250), FAST)
        .await
        .unwrap();

    assert_eq!(calls.len(), 5);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_429_wall_gives_up_after_the_budget() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let err = fetch_calls_at(&server.uri(), "k", &opts(250), FAST)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("5 retries"), "error was: {err}");
    // The first attempt plus five retries.
    assert_eq!(server.received_requests().await.unwrap().len(), 6);
}

#[tokio::test]
async fn a_401_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = fetch_calls_at(&server.uri(), "k", &opts(250), FAST)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("401"), "error was: {err}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_key_travels_in_the_header_and_never_in_the_url() {
    const KEY: &str = "sk-secret-do-not-leak";
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(1000, 2)))
        .mount(&server)
        .await;

    fetch_calls_at(&server.uri(), KEY, &opts(250), FAST)
        .await
        .unwrap();

    for req in server.received_requests().await.unwrap() {
        assert!(!req.url.as_str().contains(KEY), "key leaked into the URL");
    }
}

/// The one rule this whole file exists to keep: Vapi is read-only, forever.
#[test]
fn the_client_can_only_send_get() {
    let src = include_str!("../src/vapi.rs");
    for verb in [".post(", ".patch(", ".delete(", ".put("] {
        assert!(!src.contains(verb), "vapi.rs must never call {verb}");
    }
    assert!(src.contains(".get("), "vapi.rs stopped making requests at all");
}
