//! The saved dashboard layout: `GET` and `PUT /api/dashboard?org=`.
//!
//! The layout is a preference and nothing else. It names chart ids and says nothing about
//! what they draw, which is what lets the dashboard grow a chart — an upgrade, or a
//! structured key that arrived with the last sync — without the engine knowing anything
//! about it. So the assertions here are about shape and survival, never about contents.

use axum::serve;
use graphify::auth::Auth;
use graphify::db::{Call, Db};
use graphify::secrets::Secrets;
use graphify::server::{router, App};
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::TempDir;

/// The live server, plus the path to the file behind it, so a test can reach past the API
/// and write the way a sync does.
struct Server {
    _dir: TempDir,
    base: String,
    path: PathBuf,
    org: i64,
}

impl Server {
    /// A second connection to the same file. The server holds its own; this is how a test
    /// plays the part of the sync, which is a separate writer too.
    fn db(&self) -> Db {
        Db::open(&self.path).unwrap()
    }
}

async fn serve_one_org() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let db = Db::open(&path).unwrap();
    let org = db.create_org("acme").unwrap();

    let store = Secrets::open(dir.path().join(".secret")).unwrap();
    let app = App::new(db, store, Auth::new(None));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { serve(listener, router(app)).await.unwrap() });
    Server {
        _dir: dir,
        base: format!("http://{addr}"),
        path,
        org,
    }
}

async fn get(server: &Server, path: &str) -> (u16, Value) {
    let res = reqwest::get(format!("{}{path}", server.base)).await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn put(server: &Server, path: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new()
        .put(format!("{}{path}", server.base))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn layout(charts: &[(&str, bool)]) -> Value {
    json!({
        "charts": charts.iter().map(|(id, on)| json!({ "id": id, "on": on })).collect::<Vec<_>>()
    })
}

/// An org that has never chosen gets an empty list — which the dashboard reads as "draw
/// everything", not as "draw nothing". Anything else here would mean a fresh install shows
/// a blank page.
#[tokio::test]
async fn an_org_that_has_saved_nothing_gets_an_empty_layout() {
    let server = serve_one_org().await;
    let (status, body) = get(&server, &format!("/api/dashboard?org={}", server.org)).await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({ "charts": [] }));
}

/// The acceptance, at the API: what was turned off comes back off. The page reload the
/// spec names is this request, made again.
#[tokio::test]
async fn a_disabled_chart_comes_back_disabled() {
    let server = serve_one_org().await;
    let path = format!("/api/dashboard?org={}", server.org);
    let sent = layout(&[("cost", false), ("tokens", true)]);

    let (status, echoed) = put(&server, &path, sent.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(echoed, sent, "the response is what was stored");

    let (status, body) = get(&server, &path).await;
    assert_eq!(status, 200);
    assert_eq!(body, sent);
    assert_eq!(body["charts"][0]["on"], json!(false));
}

/// Order is the order of the list, so it has to survive the round trip exactly — a layout
/// that came back sorted would silently undo every drag.
#[tokio::test]
async fn the_order_of_the_list_is_the_order_that_comes_back() {
    let server = serve_one_org().await;
    let path = format!("/api/dashboard?org={}", server.org);
    let sent = layout(&[("tokens", true), ("cost", true), ("ended_groups", true)]);
    put(&server, &path, sent).await;

    let (_, body) = get(&server, &path).await;
    let ids: Vec<&str> = body["charts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["tokens", "cost", "ended_groups"]);
}

/// A second save replaces the first. A layout is one preference, so there is nothing here
/// to merge — a chart dropped from the list is dropped.
#[tokio::test]
async fn saving_again_replaces_the_layout() {
    let server = serve_one_org().await;
    let path = format!("/api/dashboard?org={}", server.org);
    put(&server, &path, layout(&[("cost", true), ("tokens", true)])).await;
    put(&server, &path, layout(&[("cost", false)])).await;

    let (_, body) = get(&server, &path).await;
    assert_eq!(body, layout(&[("cost", false)]));
}

/// Two orgs, two dashboards. The layout is keyed by org, so one org's choices must not
/// show up under another's name.
#[tokio::test]
async fn each_org_keeps_its_own_layout() {
    let server = serve_one_org().await;
    let other = server.db().create_org("globex").unwrap();

    let mine = layout(&[("cost", false)]);
    put(&server, &format!("/api/dashboard?org={}", server.org), mine.clone()).await;

    let (_, theirs) = get(&server, &format!("/api/dashboard?org={other}")).await;
    assert_eq!(theirs, json!({ "charts": [] }));

    let (_, back) = get(&server, &format!("/api/dashboard?org={}", server.org)).await;
    assert_eq!(back, mine);
}

/// The must-not, checked where it would actually break: a sync writes calls and then
/// purges the ones past retention. Neither touches the layout.
#[tokio::test]
async fn a_sync_does_not_lose_the_layout() {
    let server = serve_one_org().await;
    let path = format!("/api/dashboard?org={}", server.org);
    let sent = layout(&[("cost", false), ("tokens", true)]);
    put(&server, &path, sent.clone()).await;

    {
        let mut db = server.db();
        db.upsert_call(&Call {
            id: "c-1".into(),
            org_id: server.org,
            created_at: Some("2020-01-01T00:00:00.000Z".into()),
            ..Call::default()
        })
        .unwrap();
        // Old enough that retention takes it straight back out again, which is the widest
        // the write path gets.
        assert_eq!(db.purge_calls(server.org, 14, Some(1)).unwrap(), 1);
    }

    let (_, body) = get(&server, &path).await;
    assert_eq!(body, sent);
}

/// The dashboard keys its charts by id. Two rows claiming the same one leave the order of
/// the page undefined, so the layout is refused rather than stored and half-honoured.
#[tokio::test]
async fn the_same_chart_twice_is_refused() {
    let server = serve_one_org().await;
    let path = format!("/api/dashboard?org={}", server.org);
    let (status, body) = put(&server, &path, layout(&[("cost", true), ("cost", false)])).await;
    assert_eq!(status, 400);
    assert!(
        body["error"].as_str().unwrap().contains("cost"),
        "error was: {body}"
    );

    let (_, saved) = get(&server, &path).await;
    assert_eq!(saved, json!({ "charts": [] }), "nothing was stored");
}

/// A layout is a preference, not somewhere to put data.
#[tokio::test]
async fn an_id_that_is_not_a_chart_id_is_refused() {
    let server = serve_one_org().await;
    let path = format!("/api/dashboard?org={}", server.org);

    let (blank, _) = put(&server, &path, layout(&[("  ", true)])).await;
    assert_eq!(blank, 400);

    let long = "x".repeat(201);
    let (huge, _) = put(&server, &path, layout(&[(long.as_str(), true)])).await;
    assert_eq!(huge, 400);

    let many: Vec<(String, bool)> = (0..201).map(|i| (format!("c{i}"), true)).collect();
    let many: Vec<(&str, bool)> = many.iter().map(|(id, on)| (id.as_str(), *on)).collect();
    let (crowd, _) = put(&server, &path, layout(&many)).await;
    assert_eq!(crowd, 400);
}

/// Same rule as the filters: a key the engine does not know is a typo, and answering 400
/// says so. Silently ignoring it would store a layout the caller did not send.
#[tokio::test]
async fn an_unknown_parameter_is_a_400() {
    let server = serve_one_org().await;
    let (status, body) = get(&server, &format!("/api/dashboard?org={}&window=7h", server.org)).await;
    assert_eq!(status, 400);
    assert!(
        body["error"].as_str().unwrap().contains("window"),
        "error was: {body}"
    );
}

/// A layout belongs to an org, so the org is not optional — and one that does not exist is
/// a 404 rather than a layout filed under nothing.
#[tokio::test]
async fn the_org_has_to_be_named_and_has_to_exist() {
    let server = serve_one_org().await;
    let (missing, _) = get(&server, "/api/dashboard").await;
    assert_eq!(missing, 400);

    let (unknown, _) = get(&server, "/api/dashboard?org=999").await;
    assert_eq!(unknown, 404);

    let (writing, _) = put(&server, "/api/dashboard?org=999", layout(&[("cost", true)])).await;
    assert_eq!(writing, 404);
}
