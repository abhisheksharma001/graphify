use axum::serve;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use graphify::auth::Auth;
use graphify::db::{Call, Db, ToolCall};
use graphify::secrets::Secrets;
use graphify::server::{router, App};
use serde_json::{json, Value};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `Secrets` reads `GRAPHIFY_SECRET` and every key lookup reads `VAPI_API_KEY`, and the
/// suite runs its tests in parallel threads that share one process environment. Anything
/// that touches a secret takes this first.
///
/// Async, and deliberately so: the environment is read inside the request handler, which
/// is on the far side of an `await`, so the lock has to survive one.
async fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

fn clear_env() {
    for var in ["VAPI_API_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GRAPHIFY_SECRET"] {
        std::env::remove_var(var);
    }
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn minutes_ago(n: i64) -> String {
    stamp(Utc::now() - TimeDelta::minutes(n))
}

/// A live server, its base URL, and the temp dir holding the database it reads.
struct Server {
    _dir: TempDir,
    base: String,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

/// Build a database in a temp dir, seed it with `fill`, and serve it.
async fn serve_with(
    password: Option<&str>,
    vapi_base: Option<String>,
    fill: impl FnOnce(&mut Db, i64),
) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::open(dir.path().join("graphify.db")).unwrap();
    let org = db.create_org("acme").unwrap();
    fill(&mut db, org);

    let store = Secrets::open(dir.path().join(".secret")).unwrap();
    let auth = Auth::new(password.map(str::to_string));
    let mut app = App::new(db, store, auth);
    if let Some(base) = vapi_base {
        app = app.with_vapi_base(base);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { serve(listener, router(app)).await.unwrap() });
    Server {
        _dir: dir,
        base: format!("http://{addr}"),
    }
}

async fn plain(fill: impl FnOnce(&mut Db, i64)) -> Server {
    serve_with(None, None, fill).await
}

async fn get(url: &str) -> (u16, Value) {
    let res = reqwest::get(url).await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

/// Ten calls in the last hour, three of them in `transfer-error`. The spec's fixture.
fn ten_calls(db: &mut Db, org: i64) {
    for i in 0..10 {
        let transfer = i < 3;
        db.upsert_call(&Call {
            id: format!("call-{i}"),
            org_id: org,
            assistant_id: Some(if i % 2 == 0 { "a-1".into() } else { "a-2".into() }),
            created_at: Some(minutes_ago(i64::from(i))),
            duration_s: Some(f64::from(i) + 1.0),
            cost: Some(0.10),
            ended_reason: Some(if transfer {
                "assistant-forwarded-call-failed".into()
            } else {
                "customer-ended-call".into()
            }),
            ended_group: Some(if transfer { "transfer-error" } else { "customer" }.into()),
            transferred: Some(transfer),
            tool_failures: Some(i64::from(i % 2)),
            lat_turn_p50_ms: Some(f64::from(100 + i)),
            lat_turn_p95_ms: Some(f64::from(200 + i)),
            ..Call::default()
        })
        .unwrap();
    }
}

/// The spec's first acceptance case.
#[tokio::test]
async fn stats_report_the_transfer_error_group() {
    let s = plain(ten_calls).await;

    let (status, body) = get(&s.url("/api/stats?window=1d")).await;

    assert_eq!(status, 200);
    assert_eq!(body["by_ended_group"]["transfer-error"], 3);
    assert_eq!(body["by_ended_group"]["customer"], 7);
    assert_eq!(body["totals"]["calls"], 10);
    assert_eq!(body["totals"]["transfers"], 3);
}

/// The spec's second acceptance case.
#[tokio::test]
async fn a_password_locks_the_api_until_a_session_exists() {
    let s = serve_with(Some("hunter2"), None, ten_calls).await;
    let http = reqwest::Client::new();

    let (status, _) = get(&s.url("/api/stats?window=1d")).await;
    assert_eq!(status, 401, "no session, no stats");

    let wrong = http
        .post(s.url("/api/login"))
        .json(&json!({ "password": "hunter3" }))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 401);
    assert!(
        wrong.headers().get("set-cookie").is_none(),
        "a failed login must not hand out a session"
    );

    let right = http
        .post(s.url("/api/login"))
        .json(&json!({ "password": "hunter2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(right.status().as_u16(), 200);
    let cookie = right
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.contains("HttpOnly"), "cookie was: {cookie}");
    assert!(cookie.contains("SameSite=Strict"), "cookie was: {cookie}");

    let session = cookie.split(';').next().unwrap().to_string();
    let allowed = http
        .get(s.url("/api/stats?window=1d"))
        .header("cookie", session)
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status().as_u16(), 200);
}

#[tokio::test]
async fn a_session_from_one_server_does_not_open_another() {
    let a = serve_with(Some("hunter2"), None, ten_calls).await;
    let b = serve_with(Some("hunter2"), None, ten_calls).await;
    let http = reqwest::Client::new();

    let login = http
        .post(a.url("/api/login"))
        .json(&json!({ "password": "hunter2" }))
        .send()
        .await
        .unwrap();
    let session = login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let other = http
        .get(b.url("/api/stats"))
        .header("cookie", session)
        .send()
        .await
        .unwrap();
    assert_eq!(other.status().as_u16(), 401, "sessions are per process");
}

#[tokio::test]
async fn logging_in_with_no_password_configured_is_refused() {
    let s = plain(ten_calls).await;

    let res = reqwest::Client::new()
        .post(s.url("/api/login"))
        .json(&json!({ "password": "anything" }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status().as_u16(), 400);
}

#[tokio::test]
async fn orgs_are_listed_and_created_but_never_twice() {
    let s = plain(|_, _| {}).await;
    let http = reqwest::Client::new();

    let (_, listed) = get(&s.url("/api/orgs")).await;
    assert_eq!(listed[0]["name"], "acme");

    let made = http
        .post(s.url("/api/orgs"))
        .json(&json!({ "name": "globex" }))
        .send()
        .await
        .unwrap();
    assert_eq!(made.status().as_u16(), 201);

    let again = http
        .post(s.url("/api/orgs"))
        .json(&json!({ "name": "globex" }))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status().as_u16(), 409);
}

/// The whole point of the secrets routes: a value goes in and only a tail comes back.
#[tokio::test]
async fn a_stored_key_comes_back_as_a_tail_and_never_as_a_value() {
    let _guard = env_lock().await;
    clear_env();
    let s = plain(|_, _| {}).await;
    let http = reqwest::Client::new();
    let secret = "sk-must-never-be-returned-9911";

    let put = http
        .put(s.url("/api/orgs/1/secrets/vapi"))
        .json(&json!({ "value": secret }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status().as_u16(), 200);
    let put_body = put.text().await.unwrap();
    assert!(!put_body.contains(secret), "PUT echoed the value: {put_body}");

    let res = reqwest::get(s.url("/api/orgs/1/secrets")).await.unwrap();
    let body = res.text().await.unwrap();
    assert!(!body.contains(secret), "GET returned the value: {body}");

    let listed: Value = serde_json::from_str(&body).unwrap();
    let vapi = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "vapi")
        .unwrap();
    assert_eq!(vapi["set"], true);
    assert_eq!(vapi["last4"], "9911");
    assert_eq!(listed[1]["set"], false, "the other names stay unset");
}

#[tokio::test]
async fn an_unknown_secret_name_is_refused_and_an_unknown_org_is_missing() {
    let _guard = env_lock().await;
    clear_env();
    let s = plain(|_, _| {}).await;
    let http = reqwest::Client::new();

    let bad_name = http
        .put(s.url("/api/orgs/1/secrets/stripe"))
        .json(&json!({ "value": "sk-whatever-0001" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_name.status().as_u16(), 400);

    let bad_org = http
        .put(s.url("/api/orgs/77/secrets/vapi"))
        .json(&json!({ "value": "sk-whatever-0001" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_org.status().as_u16(), 404);

    let (status, _) = get(&s.url("/api/orgs/77/secrets")).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_key_test_answers_ok_for_a_working_key() {
    let _guard = env_lock().await;
    clear_env();
    let vapi = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/assistant"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": "a-1" }])))
        .mount(&vapi)
        .await;
    let s = serve_with(None, Some(vapi.uri()), |_, _| {}).await;
    let http = reqwest::Client::new();

    http.put(s.url("/api/orgs/1/secrets/vapi"))
        .json(&json!({ "value": "sk-good-key-0001" }))
        .send()
        .await
        .unwrap();
    let (status, body) = post_empty(&http, &s.url("/api/orgs/1/test")).await;

    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["assistants"], 1);
}

/// A rejected key is an answer, not a server error: the settings screen has to show it.
#[tokio::test]
async fn the_key_test_reports_a_rejected_key_without_failing() {
    let _guard = env_lock().await;
    clear_env();
    let vapi = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/assistant"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&vapi)
        .await;
    let s = serve_with(None, Some(vapi.uri()), |_, _| {}).await;
    let http = reqwest::Client::new();

    http.put(s.url("/api/orgs/1/secrets/vapi"))
        .json(&json!({ "value": "sk-bad-key-0002" }))
        .send()
        .await
        .unwrap();
    let (status, body) = post_empty(&http, &s.url("/api/orgs/1/test")).await;

    assert_eq!(status, 200);
    assert_eq!(body["ok"], false);
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("401"), "was: {message}");
    assert!(!message.contains("sk-bad-key"), "the key leaked: {message}");
}

#[tokio::test]
async fn testing_an_org_with_no_key_says_so() {
    let _guard = env_lock().await;
    clear_env();
    let s = plain(|_, _| {}).await;

    let (status, body) =
        post_empty(&reqwest::Client::new(), &s.url("/api/orgs/1/test")).await;

    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("no Vapi key"));
}

async fn post_empty(http: &reqwest::Client, url: &str) -> (u16, Value) {
    let res = http.post(url).send().await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

/// A typo in a filter name must not quietly answer with the unfiltered set.
#[tokio::test]
async fn an_unknown_filter_is_refused_by_name() {
    let s = plain(ten_calls).await;

    let (status, body) = get(&s.url("/api/stats?assistantid=a-1")).await;

    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("assistantid"));
}

#[tokio::test]
async fn assistant_id_is_repeatable_and_means_any_of_them() {
    let s = plain(ten_calls).await;

    let (_, one) = get(&s.url("/api/calls?assistant_id=a-1")).await;
    let (_, both) = get(&s.url("/api/calls?assistant_id=a-1&assistant_id=a-2")).await;

    assert_eq!(one.as_array().unwrap().len(), 5);
    assert_eq!(both.as_array().unwrap().len(), 10);
}

#[tokio::test]
async fn the_bucket_follows_the_window() {
    let s = plain(ten_calls).await;

    let (_, day) = get(&s.url("/api/stats?window=1d")).await;
    let (_, week) = get(&s.url("/api/stats?window=7d")).await;

    assert_eq!(day["bucket_size"], "1h");
    assert_eq!(week["bucket_size"], "1d");
    assert_eq!(
        day["per_bucket"].as_array().unwrap().len(),
        25,
        "a 24-hour window covers 25 hourly buckets once both ends are included"
    );
}

/// A bucket the calls did not reach is a real gap in the chart, and an empty one costs
/// nothing rather than costing zero.
#[tokio::test]
async fn an_empty_bucket_reports_no_cost_rather_than_a_cost_of_zero() {
    let s = plain(ten_calls).await;

    let (_, body) = get(&s.url("/api/stats?window=1d")).await;
    let buckets = body["per_bucket"].as_array().unwrap();
    let empty = buckets.iter().find(|b| b["calls"] == 0).unwrap();

    assert_eq!(empty["cost"], Value::Null);
    assert_eq!(empty["duration_avg"], Value::Null);
    assert_eq!(empty["latency_p50"], Value::Null);
    // Which bucket they land in depends on where in the hour the test runs; that they are
    // all somewhere on the axis does not.
    let charted: i64 = buckets.iter().map(|b| b["calls"].as_i64().unwrap()).sum();
    assert_eq!(charted, 10);
}

/// `sum()` over no priced call is NULL in SQL and stays NULL through the API.
#[tokio::test]
async fn a_call_with_no_cost_leaves_the_total_unpriced() {
    let s = plain(|db, org| {
        db.upsert_call(&Call {
            id: "unpriced".into(),
            org_id: org,
            created_at: Some(minutes_ago(5)),
            ..Call::default()
        })
        .unwrap();
    })
    .await;

    let (_, body) = get(&s.url("/api/stats")).await;

    assert_eq!(body["totals"]["calls"], 1);
    assert_eq!(body["totals"]["cost"], Value::Null);
    assert_eq!(body["totals"]["duration_avg"], Value::Null);
}

#[tokio::test]
async fn tool_failures_are_counted_by_name_and_only_when_they_failed() {
    let s = plain(|db, org| {
        db.upsert_call(&Call {
            id: "c-1".into(),
            org_id: org,
            created_at: Some(minutes_ago(5)),
            ..Call::default()
        })
        .unwrap();
        db.replace_tool_calls(
            "c-1",
            &[
                ToolCall {
                    name: Some("lookup".into()),
                    failed: Some(true),
                    ..ToolCall::default()
                },
                ToolCall {
                    name: Some("lookup".into()),
                    failed: Some(true),
                    ..ToolCall::default()
                },
                ToolCall {
                    name: Some("book".into()),
                    failed: Some(false),
                    ..ToolCall::default()
                },
            ],
        )
        .unwrap();
    })
    .await;

    let (_, body) = get(&s.url("/api/stats")).await;

    assert_eq!(body["tool_failures_by_name"]["lookup"], 2);
    assert_eq!(body["tool_failures_by_name"]["book"], Value::Null);
}

/// The cost stack has to add up to the cost bar above it, and tokens are counts, so they
/// sum. Both are what the cost and token charts are drawn from.
#[tokio::test]
async fn the_cost_breakdown_and_the_tokens_sum_across_the_selection() {
    let s = plain(|db, org| {
        for i in 0..2 {
            db.upsert_call(&Call {
                id: format!("c-{i}"),
                org_id: org,
                created_at: Some(minutes_ago(5)),
                cost: Some(0.30),
                cost_stt: Some(0.01),
                cost_llm: Some(0.02),
                cost_tts: Some(0.03),
                cost_vapi: Some(0.04),
                cost_transport: Some(0.05),
                cost_analysis: Some(0.15),
                llm_prompt_tokens: Some(1000),
                llm_completion_tokens: Some(200),
                llm_cached_tokens: Some(50),
                ..Call::default()
            })
            .unwrap();
        }
    })
    .await;

    let (_, body) = get(&s.url("/api/stats")).await;
    let t = &body["totals"];

    assert_eq!(t["prompt_tokens"], 2000);
    assert_eq!(t["completion_tokens"], 400);
    assert_eq!(t["cached_tokens"], 100);
    let part = |k: &str| t[k].as_f64().unwrap();
    let stack = part("cost_stt")
        + part("cost_llm")
        + part("cost_tts")
        + part("cost_vapi")
        + part("cost_transport")
        + part("cost_analysis");
    assert!(
        (stack - t["cost"].as_f64().unwrap()).abs() < 1e-9,
        "the stack was {stack} against a cost of {}",
        t["cost"]
    );
}

/// A latency Vapi did not report is not a latency of zero. Averaging it in as one would
/// drag every component towards a number nothing measured, and the component charts would
/// read low for exactly the calls that went wrong.
#[tokio::test]
async fn a_latency_component_is_averaged_over_the_calls_that_reported_it() {
    let s = plain(|db, org| {
        db.upsert_call(&Call {
            id: "measured".into(),
            org_id: org,
            created_at: Some(minutes_ago(5)),
            lat_turn_avg_ms: Some(800.0),
            lat_model_avg_ms: Some(400.0),
            ..Call::default()
        })
        .unwrap();
        db.upsert_call(&Call {
            id: "silent".into(),
            org_id: org,
            created_at: Some(minutes_ago(5)),
            ..Call::default()
        })
        .unwrap();
    })
    .await;

    let (_, body) = get(&s.url("/api/stats")).await;
    let t = &body["totals"];

    assert_eq!(t["calls"], 2);
    assert_eq!(t["latency_avg"], 800.0, "one call reported 800, not two");
    assert_eq!(t["latency_model"], 400.0);
    // Nothing reported these at all, so there is no average to report.
    assert_eq!(t["latency_voice"], Value::Null);
    assert_eq!(t["latency_transcriber"], Value::Null);
    assert_eq!(t["latency_endpointing"], Value::Null);
}

/// A structured key the assistant was asked for and left null is not a column to offer.
#[tokio::test]
async fn structured_keys_skip_the_ones_that_came_back_null() {
    let s = plain(|db, org| {
        for (i, structured) in [
            r#"{"call_intent": "transfer_request", "part_number": null}"#,
            r#"{"call_intent": "price_check"}"#,
        ]
        .iter()
        .enumerate()
        {
            db.upsert_call(&Call {
                id: format!("c-{i}"),
                org_id: org,
                created_at: Some(minutes_ago(i as i64 + 1)),
                structured: Some((*structured).to_string()),
                ..Call::default()
            })
            .unwrap();
        }
    })
    .await;

    let (_, body) = get(&s.url("/api/stats")).await;

    assert_eq!(body["structured_keys"]["call_intent"], 2);
    assert_eq!(body["structured_keys"]["part_number"], Value::Null);
}

#[tokio::test]
async fn a_call_carries_its_tool_rows_and_the_recording_url_but_no_audio() {
    let s = plain(|db, org| {
        db.upsert_call(&Call {
            id: "c-1".into(),
            org_id: org,
            created_at: Some(minutes_ago(5)),
            recording_url: Some("https://storage.vapi.ai/c-1.wav".into()),
            transcript: Some("AI: hello".into()),
            slim: Some(r#"{"id":"c-1","status":"ended"}"#.into()),
            ..Call::default()
        })
        .unwrap();
        db.replace_tool_calls(
            "c-1",
            &[ToolCall {
                name: Some("lookup".into()),
                seconds_from_start: Some(4.0),
                failed: Some(false),
                ..ToolCall::default()
            }],
        )
        .unwrap();
    })
    .await;

    let (status, body) = get(&s.url("/api/calls/c-1")).await;

    assert_eq!(status, 200);
    assert_eq!(body["recording_url"], "https://storage.vapi.ai/c-1.wav");
    assert_eq!(body["tool_call_rows"][0]["name"], "lookup");
    // Parsed, not a string holding JSON: the drawer reads fields, not a blob to unwrap.
    assert_eq!(body["slim"]["status"], "ended");
}

#[tokio::test]
async fn a_missing_call_is_a_404() {
    let s = plain(ten_calls).await;

    let (status, body) = get(&s.url("/api/calls/nope")).await;

    assert_eq!(status, 404);
    assert!(body["error"].as_str().unwrap().contains("nope"));
}

#[tokio::test]
async fn assistants_list_for_the_org_and_leave_the_prompt_behind() {
    let s = plain(|db, org| {
        db.upsert_assistant(&graphify::db::Assistant {
            id: "a-1".into(),
            org_id: org,
            name: Some("Service Desk".into()),
            model: Some("gpt-4.1".into()),
            system_prompt: Some("You are a service-desk agent".into()),
            structured_schema: Some(r#"{"type":"object"}"#.into()),
            ..graphify::db::Assistant::default()
        })
        .unwrap();
    })
    .await;

    let res = reqwest::get(s.url("/api/assistants?org=1")).await.unwrap();
    let body = res.text().await.unwrap();

    assert!(
        !body.contains("service-desk agent"),
        "the picker must not carry every prompt: {body}"
    );
    let listed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(listed[0]["name"], "Service Desk");
    assert_eq!(listed[0]["model"], "gpt-4.1");
    assert_eq!(listed[0]["structured_schema"]["type"], "object");
}

#[tokio::test]
async fn an_org_filter_that_matches_nothing_returns_nothing() {
    let s = plain(ten_calls).await;

    let (status, body) = get(&s.url("/api/calls?org=99")).await;

    assert_eq!(status, 200);
    assert!(body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_window_without_a_unit_is_refused() {
    let s = plain(ten_calls).await;

    let (status, body) = get(&s.url("/api/stats?window=7")).await;

    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("window"));
}

/// Bucket size follows the span asked for, not the spelling of the ask: an explicit
/// `since` two hours back is as hourly as `window=2h`.
#[tokio::test]
async fn a_short_since_gets_hourly_buckets_without_a_window() {
    let s = plain(ten_calls).await;
    let since = stamp(Utc::now() - TimeDelta::hours(2));

    let (_, body) = get(&s.url(&format!("/api/stats?since={since}"))).await;

    assert_eq!(body["bucket_size"], "1h");
    assert_eq!(body["totals"]["calls"], 10);
}
