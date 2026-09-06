//! The dashboard side of `serve`: what comes back for a path that is not an API route.
//!
//! Which page that is depends on whether `ui/dist` existed when this was compiled, and a
//! test cannot change that after the fact. So every assertion here is written against
//! `ui::built()`, and the suite is meaningful in both worlds: a checkout with no UI (CI
//! today) and one with a built dashboard in it.

use axum::serve;
use graphify::auth::Auth;
use graphify::db::Db;
use graphify::secrets::Secrets;
use graphify::server::{router, App};
use graphify::ui;
use tempfile::TempDir;

struct Server {
    _dir: TempDir,
    base: String,
}

async fn serve_with(password: Option<&str>) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    db.create_org("acme").unwrap();
    let store = Secrets::open(dir.path().join(".secret")).unwrap();
    let app = App::new(db, store, Auth::new(password.map(str::to_string)));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { serve(listener, router(app)).await.unwrap() });
    Server {
        _dir: dir,
        base: format!("http://{addr}"),
    }
}

/// Status, content type, and body as text — the three things a page is judged on here.
async fn get(server: &Server, path: &str) -> (u16, String, String) {
    let res = reqwest::get(format!("{}{path}", server.base)).await.unwrap();
    let status = res.status().as_u16();
    let content_type = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, content_type, res.text().await.unwrap())
}

/// The spec's acceptance case, in both directions: the built `index.html` when there is
/// one, and a placeholder that says so when there is not. Never a 404.
#[tokio::test]
async fn the_root_path_is_a_page_either_way() {
    let server = serve_with(None).await;

    let (status, content_type, body) = get(&server, "/").await;

    assert_eq!(status, 200);
    assert!(content_type.starts_with("text/html"), "was: {content_type}");
    if ui::built() {
        assert!(body.contains("<html") || body.contains("<!doctype"), "was: {body}");
        assert!(!body.contains("UI not built"), "the embedded page, not the placeholder");
    } else {
        assert!(body.contains("UI not built"), "was: {body}");
    }
}

/// A deep link pasted into the address bar has to load the shell, because the route it
/// names belongs to the UI's own router and not to this one.
#[tokio::test]
async fn an_unknown_page_falls_back_to_the_shell() {
    let server = serve_with(None).await;

    let (status, _, body) = get(&server, "/calls/call-123").await;
    let (_, _, root) = get(&server, "/").await;

    assert_eq!(status, 200);
    assert_eq!(body, root);
}

/// The one path the fallback must not swallow. A typo'd endpoint answered with HTML and a
/// 200 is a bug that looks like a working request.
#[tokio::test]
async fn an_unknown_api_route_is_a_404_in_json() {
    let server = serve_with(None).await;

    let (status, content_type, body) = get(&server, "/api/nope").await;
    // `/api` itself, with no route under it, is the same mistake spelled shorter.
    assert_eq!(get(&server, "/api").await.0, 404);

    assert_eq!(status, 404);
    assert!(content_type.starts_with("application/json"), "was: {content_type}");
    assert!(body.contains("error"), "was: {body}");
    assert!(!body.contains("<html"), "was: {body}");
}

/// The page is not behind the gate and the data is. The login form has to render before
/// there is a session to render it with.
#[tokio::test]
async fn the_page_loads_without_a_session_but_the_data_does_not() {
    let server = serve_with(Some("letmein")).await;

    assert_eq!(get(&server, "/").await.0, 200);
    assert_eq!(get(&server, "/api/stats?org=1").await.0, 401);
    // Including the notices board. A notice quotes SQLite about the operator's own
    // database, which is not something a stranger at the port gets to read.
    assert_eq!(get(&server, "/api/notices").await.0, 401);
}

/// Only meaningful once a build exists. Until then it asserts nothing, which is honest:
/// there is no asset to ask for.
#[tokio::test]
async fn an_embedded_asset_keeps_its_own_content_type() {
    if !ui::built() {
        return;
    }
    let server = serve_with(None).await;
    let (_, _, index) = get(&server, "/").await;

    // Whatever script tag the build wrote, fetched back by its own URL.
    let src = index
        .split("src=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("a built index.html loads a script");
    let (status, content_type, _) = get(&server, src).await;

    assert_eq!(status, 200, "asked for {src}");
    assert!(content_type.starts_with("text/javascript"), "was: {content_type}");
}
