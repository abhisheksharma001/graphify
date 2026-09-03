use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use graphify::db::Db;
use graphify::sync::{run, Opts};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// One instant for the whole process, so a `createdAtLt` cursor a mock is mounted on is
/// the same string the client later sends.
fn base() -> DateTime<Utc> {
    static BASE: OnceLock<DateTime<Utc>> = OnceLock::new();
    *BASE.get_or_init(Utc::now)
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `createdAt` for call `seq`. Higher `seq` is newer, and the strings are fixed width, so
/// string order is time order — what both the paging cursor and the `since` cutoff need.
fn at(seq: u32) -> String {
    stamp(base() - TimeDelta::minutes(i64::from(2000 - seq)))
}

fn days_ago(days: i64) -> String {
    stamp(base() - TimeDelta::days(days))
}

/// A page of `n` calls counting down from `top`, newest first, as Vapi returns them.
fn page(top: u32, n: u32) -> Value {
    let calls: Vec<Value> = (0..n)
        .map(|i| {
            let seq = top - i;
            json!({ "id": format!("call-{seq:04}"), "createdAt": at(seq) })
        })
        .collect();
    json!(calls)
}

/// Answer every `GET /call` with the same body. Fine whenever the body is a short page,
/// which ends the walk after one request.
async fn serve(body: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/call"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    serve_empty_catalog(&server).await;
    server
}

/// `sync` refreshes tools and assistants before it fetches a single call, so every server
/// here has to answer those too. Empty is the honest body: these tests are about calls,
/// and `engine/tests/assistants.rs` is where the catalog is checked.
async fn serve_empty_catalog(server: &MockServer) {
    for resource in ["/tool", "/assistant", "/squad"] {
        Mock::given(method("GET"))
            .and(path(resource))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(server)
            .await;
    }
}

/// How many times the mock was asked for calls, ignoring the catalog refresh.
async fn call_requests(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/call")
        .count()
}

/// 250 calls over three pages, the third one short.
async fn serve_250() -> MockServer {
    let server = MockServer::start().await;
    for (cursor, body) in [
        (None, page(1000, 100)),
        (Some(at(901)), page(900, 100)),
        (Some(at(801)), page(800, 50)),
    ] {
        let mock = Mock::given(method("GET")).and(path("/call"));
        // Without the `is_missing` arm the cursorless mock would also answer the cursored
        // requests, and the walk would fetch page one three times over.
        let mock = match cursor {
            Some(c) => mock.and(query_param("createdAtLt", c)),
            None => mock.and(query_param_is_missing("createdAtLt")),
        };
        mock.respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    serve_empty_catalog(&server).await;
    server
}

struct Fixture {
    _dir: TempDir,
    path: PathBuf,
    db: Db,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let db = Db::open(&path).unwrap();
    db.create_org("acme").unwrap();
    Fixture {
        _dir: dir,
        path,
        db,
    }
}

impl Fixture {
    fn sql(&self, stmt: &str) {
        Connection::open(&self.path).unwrap().execute(stmt, []).unwrap();
    }

    fn count(&self, table: &str) -> i64 {
        Connection::open(&self.path)
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn ids(&self) -> Vec<String> {
        Connection::open(&self.path)
            .unwrap()
            .prepare("SELECT id FROM calls ORDER BY created_at DESC")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
}

fn opts(server: &MockServer, last: usize) -> Opts {
    Opts {
        org: "acme".into(),
        last,
        since: None,
        base: server.uri(),
        key: "k".into(),
    }
}

/// The spec's acceptance case.
#[tokio::test]
async fn a_second_run_over_the_same_calls_adds_nothing() {
    let mut f = fixture();
    let server = serve_250().await;

    let first = run(&mut f.db, &opts(&server, 250)).await.unwrap();
    assert_eq!(first.to_string(), "org acme: synced 250 new, 250 total, purged 0");

    let second = run(&mut f.db, &opts(&server, 250)).await.unwrap();
    assert_eq!(second.to_string(), "org acme: synced 0 new, 250 total, purged 0");
    assert_eq!(f.count("calls"), 250);
    assert_eq!(
        call_requests(&server).await,
        3,
        "the target was already met, so the second run must not fetch calls at all"
    );
}

/// `--last` is a target, not a page size: 250 stored and 300 asked for means a budget of
/// 50, and the `since` cutoff drops the calls already held.
#[tokio::test]
async fn a_bigger_last_fetches_only_the_shortfall() {
    let mut f = fixture();
    let old = serve_250().await;
    run(&mut f.db, &opts(&old, 250)).await.unwrap();

    // 40 calls overlapping the stored range: 20 newer, 20 already held.
    let fresh = serve(page(1020, 40)).await;
    let report = run(&mut f.db, &opts(&fresh, 300)).await.unwrap();

    assert_eq!(report.new, 20);
    assert_eq!(report.total, 270);
    assert_eq!(f.ids()[0], "call-1020");
}

/// An explicit date is a range, so stored rows do not eat the budget.
#[tokio::test]
async fn an_explicit_since_ignores_what_is_already_stored() {
    let mut f = fixture();
    let old = serve_250().await;
    run(&mut f.db, &opts(&old, 250)).await.unwrap();

    let fresh = serve(page(1020, 40)).await;
    let report = run(
        &mut f.db,
        &Opts {
            since: Some(at(1010)),
            ..opts(&fresh, 250)
        },
    )
    .await
    .unwrap();

    assert_eq!(report.new, 10, "only calls newer than the given instant");
    assert_eq!(report.total, 260);
}

#[tokio::test]
async fn keep_days_above_the_cap_refuses_before_any_request() {
    let mut f = fixture();
    f.sql("UPDATE orgs SET keep_days = 20 WHERE name = 'acme'");
    let server = serve(page(1000, 5)).await;

    let err = run(&mut f.db, &opts(&server, 250)).await.unwrap_err();

    assert!(err.to_string().contains("14-day"), "was: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a bad retention setting must not cost a request"
    );
}

#[tokio::test]
async fn the_age_sweep_spares_the_keep_window() {
    let mut f = fixture();
    let server = serve(json!([
        { "id": "recent", "createdAt": days_ago(1) },
        { "id": "edge",   "createdAt": days_ago(13) },
        { "id": "stale",  "createdAt": days_ago(20) },
    ]))
    .await;

    let report = run(&mut f.db, &opts(&server, 250)).await.unwrap();

    assert_eq!(report.new, 3, "purge runs after the write, not instead of it");
    assert_eq!(report.purged, 1);
    assert_eq!(report.total, 2);
    assert_eq!(f.ids(), vec!["recent".to_string(), "edge".to_string()]);
}

/// An unknown age is not an old age.
#[tokio::test]
async fn a_call_with_no_created_at_is_never_swept_by_age() {
    let mut f = fixture();
    let server = serve(json!([{ "id": "undated" }])).await;

    let report = run(&mut f.db, &opts(&server, 250)).await.unwrap();

    assert_eq!(report.purged, 0);
    assert_eq!(report.total, 1);
}

#[tokio::test]
async fn max_calls_keeps_the_newest() {
    let mut f = fixture();
    f.sql("UPDATE orgs SET max_calls = 2 WHERE name = 'acme'");
    let server = serve(page(1000, 5)).await;

    let report = run(&mut f.db, &opts(&server, 250)).await.unwrap();

    assert_eq!(report.purged, 3);
    assert_eq!(report.total, 2);
    assert_eq!(f.ids(), vec!["call-1000".to_string(), "call-0999".to_string()]);
}

#[tokio::test]
async fn purging_a_call_takes_its_tool_rows_with_it() {
    let mut f = fixture();
    let server = serve(json!([
        { "id": "recent", "createdAt": days_ago(1),  "artifact": { "messages": messages("lookup") } },
        { "id": "stale",  "createdAt": days_ago(20), "artifact": { "messages": messages("book") } },
    ]))
    .await;

    let report = run(&mut f.db, &opts(&server, 250)).await.unwrap();

    assert_eq!(report.purged, 1);
    assert_eq!(f.count("tool_calls"), 1, "the purged call must not leave orphans");
}

fn messages(tool: &str) -> Value {
    json!([{
        "role": "tool_calls",
        "toolCalls": [{ "id": "t1", "function": { "name": tool, "arguments": "{}" } }],
    }])
}

/// `extract` leaves `synced_at` NULL on purpose; filling it is sync's job.
#[tokio::test]
async fn sync_stamps_the_row_the_extractor_left_null() {
    let mut f = fixture();
    let server = serve(page(1000, 3)).await;

    run(&mut f.db, &opts(&server, 250)).await.unwrap();

    let unstamped = Connection::open(&f.path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM calls WHERE synced_at IS NULL",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(unstamped, 0);
}

#[tokio::test]
async fn an_unknown_org_is_an_error() {
    let mut f = fixture();
    let server = serve(page(1000, 5)).await;

    let err = run(
        &mut f.db,
        &Opts {
            org: "globex".into(),
            ..opts(&server, 250)
        },
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("globex"), "was: {err}");
}
