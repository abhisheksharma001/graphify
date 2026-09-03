//! The dashboard, served out of the binary.
//!
//! `build.rs` embeds `ui/dist` when there is one. When there is not — a fresh checkout, or
//! CI before the UI exists — the table is empty and every page path answers with one
//! placeholder saying so. A 200 that explains itself beats a 404 that reads like a broken
//! route.
//!
//! None of this sits behind the password gate. The login form has to render before there
//! is a session to render it with, and a bundle holds nothing worth protecting: the data
//! is all behind `/api`.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// Whether a built UI was embedded. A test can only assert the branch it was compiled
/// with, so it has to be able to ask which one that is.
pub fn built() -> bool {
    !ASSETS.is_empty()
}

const PLACEHOLDER: &str = r#"<!doctype html>
<title>graphify</title>
<style>body{font:16px/1.5 system-ui,sans-serif;margin:4rem auto;max-width:34rem;padding:0 1rem}
code{background:#eee;padding:.1rem .3rem;border-radius:3px}</style>
<h1>UI not built</h1>
<p>The API is running. The dashboard is not in this binary because
<code>ui/dist</code> did not exist when it was compiled.</p>
<p>Build it and rebuild the engine:</p>
<pre><code>cd ui && pnpm i && pnpm build
cd ../engine && cargo build --release</code></pre>
<p>The API is up either way — try <code>/api/orgs</code>.</p>
"#;

/// Everything the routes above did not claim.
pub async fn handler(uri: Uri) -> Response {
    // An unknown API route is a missing route, not a page. Answering it with the shell
    // would hand HTML to a caller that asked for JSON and call that a success.
    if uri.path() == "/api" || uri.path().starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such endpoint" })),
        )
            .into_response();
    }
    match file(uri.path().trim_start_matches('/')) {
        Some(res) => res,
        // Every other path belongs to the UI's own router, so hand back the shell and let
        // it decide. A deep link pasted into the address bar has to load something.
        None => index(),
    }
}

/// A linear scan, because a Vite build is a few dozen files and an index would cost more
/// to build on every start than it saves on any request.
///
/// Nothing here touches the filesystem, so a path cannot climb out of the folder: a name
/// either matches something the compiler put in the table or it matches nothing.
fn file(path: &str) -> Option<Response> {
    let (_, bytes) = ASSETS.iter().find(|(p, _)| *p == path)?;
    Some(([(header::CONTENT_TYPE, content_type(path))], *bytes).into_response())
}

fn index() -> Response {
    file("index.html").unwrap_or_else(|| {
        (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            PLACEHOLDER,
        )
            .into_response()
    })
}

/// Vite writes a short and known set of extensions, so a guessing crate would be a
/// dependency taken on to answer questions this is never asked.
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}
