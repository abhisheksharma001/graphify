//! The HTTP API. Ten routes over the local database, plus a login when a password is
//! configured, plus the dashboard itself on everything left over. Everything here is
//! read-only against Vapi and write-only against SQLite: the only outbound request the
//! whole file can make is the connectivity test, and that one is a `GET`.
//!
//! Bound to loopback unless told otherwise. A dashboard that reaches a Vapi key is not
//! something to put on `0.0.0.0` because a default said so.

use crate::auth::{self, Auth};
use crate::db::Db;
use crate::queries::{self, Filters};
use crate::secrets::{self, Secrets};
use crate::ui;
use crate::sync;
use crate::vapi::{self, Retry};
use anyhow::Result;
use axum::extract::{Path, RawQuery, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

/// Loopback, always, unless `GRAPHIFY_BIND` says otherwise.
pub const DEFAULT_BIND: &str = "127.0.0.1:3737";

/// A connectivity test answers now or not at all. The retry policy that serves a sync
/// well would leave the settings screen spinning for fifteen seconds on a dead endpoint.
const TEST_RETRY: Retry = Retry {
    max: 0,
    base_ms: 0,
};

/// Everything a handler needs. `Db` holds a `rusqlite::Connection`, which is not `Sync`,
/// so it lives behind a mutex; the critical sections are single statements and the one
/// handler that waits on the network takes what it needs and lets go first.
#[derive(Clone)]
pub struct App {
    db: Arc<Mutex<Db>>,
    secrets: Arc<Secrets>,
    auth: Arc<Auth>,
    /// Where the connectivity test points. Configurable so tests can aim at a mock.
    vapi_base: String,
}

impl App {
    pub fn new(db: Db, secrets: Secrets, auth: Auth) -> Self {
        App {
            db: Arc::new(Mutex::new(db)),
            secrets: Arc::new(secrets),
            auth: Arc::new(auth),
            vapi_base: vapi::DEFAULT_BASE.to_string(),
        }
    }

    pub fn with_vapi_base(mut self, base: impl Into<String>) -> Self {
        self.vapi_base = base.into();
        self
    }

    /// A poisoned lock means some other handler panicked mid-statement. The database is
    /// still consistent — SQLite saw either a whole statement or none of it — so recover
    /// rather than poison every later request too.
    fn db(&self) -> MutexGuard<'_, Db> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub fn bind_addr() -> String {
    std::env::var("GRAPHIFY_BIND")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BIND.to_string())
}

/// Serve until the process is stopped, printing the address actually bound.
pub async fn serve(app: App, addr: &str, open: bool) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let url = format!("http://{}", listener.local_addr()?);
    println!("graphify listening on {url}");
    // After the bind, so the tab that opens finds something listening.
    if open {
        open_browser(&url);
    }
    axum::serve(listener, router(app)).await?;
    Ok(())
}

/// Best effort, and deliberately so: a container or a headless box has no browser to open,
/// and that is not a reason to refuse to serve. The address went to stdout either way.
fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

pub fn router(app: App) -> Router {
    // The gate goes on this router and not on the login, which is the one route a caller
    // with no session has to be able to reach.
    let guarded = Router::new()
        .route("/api/orgs", get(list_orgs).post(create_org))
        .route("/api/orgs/{id}", put(set_limits))
        .route("/api/orgs/{id}/secrets", get(list_secrets))
        .route("/api/orgs/{id}/secrets/{name}", put(set_secret))
        .route("/api/orgs/{id}/test", post(test_key))
        .route("/api/secrets", get(list_global_secrets))
        .route("/api/secrets/{name}", put(set_global_secret))
        .route("/api/assistants", get(list_assistants))
        .route("/api/calls", get(list_calls))
        .route("/api/calls/{id}", get(get_call))
        .route("/api/stats", get(get_stats))
        .route("/api/dashboard", get(get_dashboard).put(put_dashboard))
        .layer(middleware::from_fn_with_state(app.clone(), gate));

    Router::new()
        .route("/api/login", post(login))
        .merge(guarded)
        // Outside the gate: the login form has to render before there is a session.
        .fallback(ui::handler)
        .with_state(app)
}

async fn gate(State(app): State<App>, req: Request, next: Next) -> Result<Response, ApiError> {
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if !app.auth.allows(cookie.as_deref()) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "not signed in"));
    }
    Ok(next.run(req).await)
}

#[derive(Deserialize)]
struct LoginBody {
    password: String,
}

async fn login(State(app): State<App>, Json(body): Json<LoginBody>) -> Result<Response, ApiError> {
    if !app.auth.required() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "no password is configured, so there is nothing to sign in to",
        ));
    }
    let Some(token) = app.auth.login(&body.password) else {
        // One message for a wrong password, whatever was wrong with it.
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "wrong password"));
    };
    Ok((
        [(header::SET_COOKIE, auth::set_cookie(&token))],
        Json(json!({ "ok": true })),
    )
        .into_response())
}

async fn list_orgs(State(app): State<App>) -> Result<Response, ApiError> {
    let orgs = app.db().list_orgs()?;
    Ok(Json(orgs).into_response())
}

#[derive(Deserialize)]
struct NewOrg {
    name: String,
}

async fn create_org(
    State(app): State<App>,
    Json(body): Json<NewOrg>,
) -> Result<Response, ApiError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "an org needs a name"));
    }
    let db = app.db();
    // Checked rather than caught: a UNIQUE violation surfacing as a 500 would tell the
    // person typing the name nothing about what to do next.
    if db.org_by_name(&name)?.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("an org named {name} already exists"),
        ));
    }
    let id = db.create_org(&name)?;
    // The stored row, not an echo of the request: the caller gets the retention defaults
    // it did not send, and the settings screen gets the same shape `/api/orgs` returns.
    Ok((StatusCode::CREATED, Json(db.org_by_id(id)?)).into_response())
}

/// The retention settings, and only those: a name is what an org is known by elsewhere,
/// so renaming one is not something the settings screen does by accident.
#[derive(Deserialize)]
struct Limits {
    keep_days: Option<i64>,
    max_calls: Option<i64>,
}

async fn set_limits(
    State(app): State<App>,
    Path(id): Path<i64>,
    Json(body): Json<Limits>,
) -> Result<Response, ApiError> {
    // D-5 is a cap, not a default: an org may keep less, never more. Refused here as well
    // as in `sync`, so the number is rejected while someone is looking at it rather than
    // at the next unattended run.
    if body
        .keep_days
        .is_some_and(|d| !(1..=sync::MAX_KEEP_DAYS).contains(&d))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("keep_days must be between 1 and {}", sync::MAX_KEEP_DAYS),
        ));
    }
    if body.max_calls.is_some_and(|n| n < 1) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "max_calls must be at least 1, or absent for no limit",
        ));
    }
    let db = app.db();
    known_org(&db, id)?;
    db.set_org_limits(id, body.keep_days, body.max_calls)?;
    // The stored row, so the caller never has to assume what landed.
    Ok(Json(db.org_by_id(id)?).into_response())
}

/// Which secrets this org has, and their last four characters. Never their values.
async fn list_secrets(State(app): State<App>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let db = app.db();
    known_org(&db, id)?;
    Ok(Json(app.secrets.status(&db, Some(id))?).into_response())
}

#[derive(Deserialize)]
struct NewSecret {
    value: String,
}

async fn set_secret(
    State(app): State<App>,
    Path((id, name)): Path<(i64, String)>,
    Json(body): Json<NewSecret>,
) -> Result<Response, ApiError> {
    let value = named(&secrets::ORG_NAMES, &name, &body)?;
    let db = app.db();
    known_org(&db, id)?;
    app.secrets.set(&db, Some(id), &name, value)?;
    // The new status, not the value: this response goes straight back to a browser.
    Ok(Json(app.secrets.status(&db, Some(id))?).into_response())
}

/// The install's own keys — the model providers — which no org owns.
async fn list_global_secrets(State(app): State<App>) -> Result<Response, ApiError> {
    let db = app.db();
    Ok(Json(app.secrets.status(&db, None)?).into_response())
}

async fn set_global_secret(
    State(app): State<App>,
    Path(name): Path<String>,
    Json(body): Json<NewSecret>,
) -> Result<Response, ApiError> {
    let value = named(&secrets::GLOBAL_NAMES, &name, &body)?;
    let db = app.db();
    app.secrets.set(&db, None, &name, value)?;
    Ok(Json(app.secrets.status(&db, None)?).into_response())
}

/// The two checks both secret routes owe: that the name belongs at this scope, and that
/// the value is not blank. Returns the trimmed value, which is what gets stored.
fn named<'a>(allowed: &[&str], name: &str, body: &'a NewSecret) -> Result<&'a str, ApiError> {
    if !allowed.contains(&name) {
        // Named against this scope, not against every secret there is: "anthropic" is a
        // real key and putting it on an org is still the wrong place for it.
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "unknown secret {name} here; expected one of {}",
                allowed.join(", ")
            ),
        ));
    }
    let value = body.value.trim();
    if value.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "a secret needs a value",
        ));
    }
    Ok(value)
}

/// One `GET /assistant` with the org's key, to answer "is this key any good".
async fn test_key(State(app): State<App>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    // The lock is taken and dropped before the request goes out: no handler waits on the
    // network while holding the database.
    let key = {
        let db = app.db();
        known_org(&db, id)?;
        app.secrets.get(&db, Some(id), "vapi")?
    };
    let Some(key) = key else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "this org has no Vapi key",
        ));
    };

    // A failure is an answer, not an error: "your key does not work" is what was asked.
    // Vapi's own message is safe to pass on — the key travels in a header, never in the
    // URL or the body that a message could quote back.
    Ok(
        match vapi::fetch_all_at(&app.vapi_base, key.expose(), "assistant", TEST_RETRY).await {
            Ok(list) => Json(json!({ "ok": true, "assistants": list.len() })),
            Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
        }
        .into_response(),
    )
}

async fn list_assistants(
    State(app): State<App>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let filters = filters(query.as_deref())?;
    Ok(Json(queries::assistants(&app.db(), filters.org)?).into_response())
}

async fn list_calls(
    State(app): State<App>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let filters = filters(query.as_deref())?;
    Ok(Json(queries::calls(&app.db(), &filters)?).into_response())
}

async fn get_call(State(app): State<App>, Path(id): Path<String>) -> Result<Response, ApiError> {
    match queries::call(&app.db(), &id)? {
        Some(call) => Ok(Json(call).into_response()),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no call {id}"),
        )),
    }
}

async fn get_stats(
    State(app): State<App>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let filters = filters(query.as_deref())?;
    Ok(Json(queries::stats(&app.db(), &filters)?).into_response())
}

/// One chart of the dashboard, and whether it is drawn. The order of the list is the order
/// the charts appear in.
///
/// The ids belong to the dashboard, not to the engine. Most of them name a chart that has
/// existed since it was compiled, but a structured key becomes a chart the moment a call
/// carries it, so there is no closed set of them to check against. The engine stores the
/// list it is handed and checks its shape rather than its contents.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChartPref {
    pub id: String,
    pub on: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub charts: Vec<ChartPref>,
}

impl Layout {
    /// What an org with nothing saved gets back. Read by the dashboard as "no choice has
    /// been made", so it draws every chart it has — not as "draw nothing".
    fn none() -> Self {
        Layout { charts: Vec::new() }
    }
}

/// A layout is a preference, not somewhere to put data. Room for every chart the dashboard
/// draws plus every structured key an org could plausibly be collecting.
const MAX_CHARTS: usize = 200;
const MAX_ID: usize = 200;

/// The saved layout for one org, or an empty one.
async fn get_dashboard(
    State(app): State<App>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let org = org_param(query.as_deref())?;
    let db = app.db();
    known_org(&db, org)?;
    let layout = match db.dashboard(org)? {
        // A layout that will not parse is a preference that cannot be honoured, and the
        // answer to that is the default dashboard — not a 500 that takes every chart down
        // with it because of a row nobody can see.
        Some(json) => serde_json::from_str(&json).unwrap_or_else(|_| Layout::none()),
        None => Layout::none(),
    };
    Ok(Json(layout).into_response())
}

async fn put_dashboard(
    State(app): State<App>,
    RawQuery(query): RawQuery,
    Json(layout): Json<Layout>,
) -> Result<Response, ApiError> {
    let org = org_param(query.as_deref())?;
    check(&layout)?;
    let db = app.db();
    known_org(&db, org)?;
    let json = serde_json::to_string(&layout).map_err(anyhow::Error::from)?;
    db.set_dashboard(org, &json)?;
    // Back comes what was stored, so a caller never has to assume it landed.
    Ok(Json(layout).into_response())
}

/// Shape, not contents. A duplicate id is the one thing worth refusing outright: the
/// dashboard keys its charts by id, so two rows claiming the same one leave the order of
/// the page undefined.
fn check(layout: &Layout) -> Result<(), ApiError> {
    if layout.charts.len() > MAX_CHARTS {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("a layout holds at most {MAX_CHARTS} charts"),
        ));
    }
    let mut seen = HashSet::new();
    for chart in &layout.charts {
        if chart.id.trim().is_empty() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "a chart in the layout needs an id",
            ));
        }
        if chart.id.len() > MAX_ID {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("chart ids are at most {MAX_ID} characters"),
            ));
        }
        if !seen.insert(chart.id.as_str()) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("chart {} is in the layout twice", chart.id),
            ));
        }
    }
    Ok(())
}

/// The org a layout belongs to, read by hand rather than through `Filters`. A layout
/// belongs to an org and not to a selection: accepting `?window=7h` here would say it
/// could differ per range, which it cannot.
fn org_param(query: Option<&str>) -> Result<i64, ApiError> {
    let mut org = None;
    for (k, v) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match k.as_ref() {
            "org" => {
                org = Some(v.parse::<i64>().map_err(|_| {
                    ApiError::new(StatusCode::BAD_REQUEST, "org must be an org id")
                })?)
            }
            other => {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("unknown parameter {other}"),
                ))
            }
        }
    }
    org.ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "a dashboard layout belongs to an org, so ?org= is required",
        )
    })
}

/// A bad filter is the caller's mistake, so it is a 400 carrying the reason — a typo in a
/// filter name must not come back as an unexplained 500.
fn filters(query: Option<&str>) -> Result<Filters, ApiError> {
    Filters::from_query(query.unwrap_or_default())
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))
}

/// Checked before a secret is written or read, so a typo'd org id is a 404 rather than a
/// secret quietly filed under an org that does not exist.
fn known_org(db: &Db, id: i64) -> Result<(), ApiError> {
    match db.org_by_id(id)? {
        Some(_) => Ok(()),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no org {id}"),
        )),
    }
}

/// Every failure the API can return, as one status and one message.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError {
            status,
            message: message.into(),
        }
    }
}

/// Anything that got this far without being classified is the server's fault.
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
