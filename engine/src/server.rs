//! The HTTP API. The routes over the local database, plus a login when a password is
//! configured, plus the dashboard itself on everything left over. Everything here is
//! read-only against Vapi and write-only against SQLite: the only outbound request the
//! whole file can make is the connectivity test, and that one is a `GET`.
//!
//! The pattern routes start a brain subprocess and read its row back; the spawning itself
//! is `jobs`'.
//!
//! Bound to loopback unless told otherwise. A dashboard that reaches a Vapi key is not
//! something to put on `0.0.0.0` because a default said so.

use crate::ask;
use crate::auth::{self, Auth};
use crate::db::Db;
use crate::jobs::{self, Jobs, Kind};
use crate::queries::{self, Filters};
use crate::rules;
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
use serde_json::{json, Value};
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
    /// The brain jobs that are parked on their go. Shared, because the thread holding a
    /// child and the handler that wakes it are on opposite sides of the process.
    jobs: Arc<Jobs>,
    /// The brain binary to spawn. Read from the environment once here rather than at each
    /// spawn, so a test can point one server at a fake without touching the environment
    /// every other test in the process is also reading.
    brain: String,
}

/// Clear whatever the last process left live, and hand back what the operator has to be
/// told if it could not be done.
///
/// Returns the sentence rather than a `Result` because the sentence is the part worth
/// proving. That SQLite can refuse an `UPDATE` is not in doubt; what matters is that the
/// person running this finds out the queue has been disabled, and finds it out in words
/// that say so rather than in a rusqlite error about a statement they did not write.
///
/// Nothing here is scrubbed and nothing needs to be, on S-37's rule: this error comes from
/// SQLite about the engine's own statement, and carries no key, no brain output and no
/// operator text.
pub fn sweep_abandoned(db: &Db) -> Option<String> {
    match db.abandon_live_jobs(jobs::RUNNING, jobs::WAITING, jobs::EXPIRED, &crate::now()) {
        Ok(_) => None,
        Err(e) => Some(format!(
            "could not clear the jobs left behind by the last run: {e:#}. They still count \
             against the limit of {}, so new jobs may be refused until this database is \
             writable and graphify is started again.",
            jobs::MAX_LIVE
        )),
    }
}

impl App {
    pub fn new(db: Db, secrets: Secrets, auth: Auth) -> Self {
        // Whatever was running or waiting belonged to a process that is gone, and its
        // children went with it. Left alone, four abandoned `waiting` rows would count
        // against the live cap for ever and no job would ever start again — `live_jobs`
        // counts rows and not children, and the next boot runs this same sweep against the
        // same database, so a failure here does not heal on a restart.
        //
        // Said and not obeyed: serving is still right, because what is lost is the job
        // queue and not the product — charts, sync, patterns and settings read other
        // tables and are unharmed — and refusing to boot would turn one feature's outage
        // into all of them on the evidence of a single `UPDATE`. The rows it could not
        // touch stay readable through the API, which is the one thing that keeps this
        // honest rather than merely quiet.
        if let Some(said) = sweep_abandoned(&db) {
            eprintln!("{said}");
        }
        App {
            db: Arc::new(Mutex::new(db)),
            secrets: Arc::new(secrets),
            auth: Arc::new(auth),
            vapi_base: vapi::DEFAULT_BASE.to_string(),
            jobs: Arc::new(Jobs::new()),
            brain: jobs::binary_from_env(),
        }
    }

    pub fn with_brain(mut self, binary: impl Into<String>) -> Self {
        self.brain = binary.into();
        self
    }

    /// How long a job parked on its go waits before it is killed unspent. Only the tests
    /// set this; everything else takes the half-hour `jobs` chose.
    pub fn with_go_wait(mut self, wait: std::time::Duration) -> Self {
        self.jobs = Arc::new(Jobs::waiting_for(wait));
        self
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
        .route("/api/assistants/{id}/prompt", get(get_assistant_prompt))
        .route("/api/calls", get(list_calls))
        .route("/api/calls/{id}", get(get_call))
        .route("/api/stats", get(get_stats))
        .route("/api/dashboard", get(get_dashboard).put(put_dashboard))
        // The four brain functions get a route each rather than one route over a captured
        // name: which functions the API can start is a decision, and a list of four is the
        // place to read it.
        .route("/api/patterns/plan", post(start_plan))
        .route("/api/patterns/clarify", post(start_clarify))
        .route("/api/patterns/label", post(start_label))
        .route("/api/patterns/synthesize", post(start_synthesize))
        // The ask box is two routes because of one rule: a price somebody walks away from
        // must leave nothing behind. `quote` starts no process and writes no row, so
        // cancelling costs a round trip; `ask` is the click, and by then the figure has
        // already been agreed.
        .route("/api/ask/quote", post(quote_ask))
        .route("/api/ask", post(start_ask))
        .route("/api/patterns", get(list_patterns))
        .route("/api/patterns/{id}", put(update_pattern))
        .route("/api/patterns/{id}/apply", post(apply_pattern))
        .route("/api/jobs/{id}", get(get_job))
        .route("/api/jobs/{id}/go", post(go_job))
        .route("/api/jobs/{id}/stop", post(stop_job))
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

/// One assistant's system prompt, asked for by name.
///
/// Not a field of the list above: the list is a picker and these prompts run to tens of
/// kilobytes each. The pattern wizard reads one, once, and only when the analyst has
/// ticked "read the agent's prompt" — so the prompt reaches the browser on a request that
/// says out loud that it wants it.
///
/// `org` is required and is part of the lookup, not a filter applied afterwards: an
/// assistant id from one client must not read a prompt out of another's.
async fn get_assistant_prompt(
    State(app): State<App>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let org = org_param(query.as_deref())?;
    let db = app.db();
    known_org(&db, org)?;
    match db.assistant_prompt(org, &id)? {
        Some(prompt) => Ok(Json(json!({ "id": id, "system_prompt": prompt })).into_response()),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no assistant {id}"),
        )),
    }
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

impl From<ask::Error> for ApiError {
    /// A refusal is something the caller can act on — an empty question, a model nobody
    /// prices, a window whose statistics do not fit — so it goes back with its own words
    /// and a 400. Anything else is the engine failing to read its own database.
    fn from(e: ask::Error) -> Self {
        match e {
            ask::Error::Refused(why) => ApiError::new(StatusCode::BAD_REQUEST, why),
            ask::Error::Failed(e) => ApiError::from(e),
        }
    }
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

// --- brain jobs -----------------------------------------------------------------------

async fn start_plan(app: State<App>, q: RawQuery, body: Json<Value>) -> Result<Response, ApiError> {
    start(Kind::Plan, app, q, body)
}

async fn start_clarify(
    app: State<App>,
    q: RawQuery,
    body: Json<Value>,
) -> Result<Response, ApiError> {
    start(Kind::Clarify, app, q, body)
}

async fn start_label(
    app: State<App>,
    q: RawQuery,
    body: Json<Value>,
) -> Result<Response, ApiError> {
    start(Kind::Label, app, q, body)
}

async fn start_synthesize(
    app: State<App>,
    q: RawQuery,
    body: Json<Value>,
) -> Result<Response, ApiError> {
    start(Kind::Synthesize, app, q, body)
}

/// Spawn one brain function and hand back the job id to watch it by.
///
/// The body is forwarded to the brain exactly as it arrived. The engine does not know what
/// a plan or a label request looks like and has no business editing one: the brain names
/// the key it did not expect, in its own words, and that message reaches the log.
///
/// The org rides in the query string rather than the body for the same reason. It is the
/// engine's own bookkeeping — `jobs` has no org column and the spend does — and putting it
/// in the body would mean adding a key the brain would then refuse.
fn start(
    kind: Kind,
    State(app): State<App>,
    RawQuery(query): RawQuery,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let org = org_param(query.as_deref())?;
    if !body.is_object() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "a brain request is a JSON object",
        ));
    }
    spawn(&app, kind, org, &body)
}

/// Check there is room for another child, then start one. Split out of `start` because the
/// ask box does not forward a body: the engine builds that request itself, out of the
/// statistics and the sample it just priced.
fn spawn(app: &App, kind: Kind, org: i64, body: &Value) -> Result<Response, ApiError> {
    {
        let db = app.db();
        known_org(&db, org)?;
        if db.live_jobs(jobs::RUNNING, jobs::WAITING)? >= jobs::MAX_LIVE {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "{} jobs are already running or waiting for a go; finish or abandon one first",
                    jobs::MAX_LIVE
                ),
            ));
        }
    }
    let id = jobs::start(&app.jobs, &app.db, &app.secrets, &app.brain, kind, org, body)?;
    // 202: the row exists and the child is starting. Everything after this is read through
    // `GET /api/jobs/{id}`.
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id, "status": jobs::RUNNING }))).into_response())
}

// --- the ask box ----------------------------------------------------------------------

/// What a question would cost. Deliberately not enough to ask one: `max_usd` is missing,
/// because a quote is a question about a price and not an approval of it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskQuote {
    question: String,
    model: String,
}

/// The same, plus the figure the person actually agreed to.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskRequest {
    question: String,
    model: String,
    /// What the quote said when it was shown. Not what the engine thinks it costs now:
    /// this is the number a person looked at, so it is the number the run may not exceed.
    max_usd: f64,
}

/// Price a question. Starts nothing, writes nothing, and holds nothing afterwards.
///
/// This route is the step's acceptance. Everywhere else the price comes from the brain,
/// which means a `jobs` row and a parked interpreter before anyone has seen a figure —
/// and four abandoned quotes would be the engine's whole job budget held by questions
/// nobody asked. Here, walking away costs one round trip and leaves no trace.
async fn quote_ask(
    State(app): State<App>,
    RawQuery(query): RawQuery,
    Json(body): Json<AskQuote>,
) -> Result<Response, ApiError> {
    let filters = filters(query.as_deref())?;
    let db = app.db();
    // A quote spends nothing, but a price for an org that does not exist is a price for
    // nothing at all — and it would come back as a plausible-looking figure rather than as
    // the typo it is.
    known_org(&db, asking_org(&filters)?)?;
    let quote = ask::quote(&db, &filters, &body.question, &body.model)?;
    Ok(Json(quote).into_response())
}

/// Which org a question is about. Required even for a quote, which spends nothing: a
/// question over every org at once is not something this dashboard shows, so a missing
/// `?org=` is a mistake to name rather than a selection to price.
fn asking_org(f: &Filters) -> Result<i64, ApiError> {
    f.org.ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "a question is about one org's calls, so ?org= is required",
        )
    })
}

/// Ask it. The click, and the only thing here that spends.
///
/// The question is priced again rather than trusting a set of ids that went to a browser
/// and came back, so what the answer is built from is what the engine chose. If the
/// selection moved in between — a sync landed, retention ran — the new price is compared
/// against the one the person approved, and a higher one stops here instead of being
/// quietly charged.
async fn start_ask(
    State(app): State<App>,
    RawQuery(query): RawQuery,
    Json(body): Json<AskRequest>,
) -> Result<Response, ApiError> {
    // The org comes out of the filter set rather than from `org_param`, which refuses any
    // key but `org`. A question arrives with the whole filter bar on it — that is what
    // says which calls it is about — so reading the org twice would mean refusing every
    // question asked over a window.
    let filters = filters(query.as_deref())?;
    let org = asking_org(&filters)?;
    // NaN fails this, which is why it is written out rather than as one comparison: a cap
    // that nothing is greater than is not a cap.
    if !body.max_usd.is_finite() || body.max_usd <= 0.0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "max_usd is the price that was approved, so it has to be a positive number",
        ));
    }
    let quote = {
        let db = app.db();
        known_org(&db, org)?;
        ask::quote(&db, &filters, &body.question, &body.model)?
    };
    if quote.usd > body.max_usd {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "this question now prices at ${:.4}, over the ${:.4} that was quoted; \
                 the calls it would read have changed, so ask for the price again",
                quote.usd, body.max_usd
            ),
        ));
    }
    // The brain is handed the statistics as the string that was priced, not a re-serialised
    // copy of them: the characters it is shown are the characters somebody paid for.
    let request = json!({
        "question": quote.question,
        "stats": quote.stats,
        "model": quote.model,
        "call_ids": quote.call_ids,
        "max_usd": body.max_usd,
    });
    spawn(&app, Kind::Ask, org, &request)
}

/// One job: where it has got to, what it quoted, and what it has cost.
///
/// The progress and the price are read back out of the log the brain wrote rather than
/// kept in columns beside it, so there is one account of what the job said and nothing to
/// fall out of step with it.
async fn get_job(State(app): State<App>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let Some(job) = app.db().job(id)? else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, format!("no job {id}")));
    };
    let progress = jobs::progress(&job.log).map(|(done, of)| json!({ "done": done, "of": of }));
    Ok(Json(json!({
        "id": job.id,
        "kind": job.kind,
        "status": job.status,
        "progress": progress,
        "estimate_usd": jobs::estimate(&job.log),
        "cost_usd": job.cost_usd,
        "output": job.output.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()),
        "log": job.log,
        "created_at": job.created_at,
        "finished_at": job.finished_at,
    }))
    .into_response())
}

/// Approve the price. This is the click, and it is the only thing in the engine that lets
/// a labelling job read a call.
async fn go_job(State(app): State<App>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    if !app.jobs.go(id) {
        // Not 404: the row may well exist. What it is not is parked on a go — it finished,
        // it expired, it was already gone, or it is a kind that never waits for one.
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("job {id} is not waiting for a go"),
        ));
    }
    Ok(Json(json!({ "id": id, "status": jobs::RUNNING })).into_response())
}

/// Turn a parked job's price down. The other answer to the same question, and the one the
/// 429 above has always told people to give without giving them anywhere to give it.
///
/// Refuses on exactly the terms `/go` refuses on, because it is the same map and the same
/// removal: a job that has finished, expired, already gone or already been stopped is not
/// waiting for an answer, and there is nothing here to say no to. Nothing running is
/// touched — that job has spent, and stopping it would be a refund the engine cannot give.
async fn stop_job(State(app): State<App>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    if !app.jobs.stop(id) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("job {id} is not waiting for an answer"),
        ));
    }
    // `expired` is what the row will say, and it is not written here: the supervisor owns
    // that write and has a child to kill first. What is true the moment this returns is
    // that the slot is nobody's.
    Ok(Json(json!({ "id": id, "status": jobs::EXPIRED })).into_response())
}

// --- patterns -------------------------------------------------------------------------

/// The org's patterns, each with how many calls of the current selection it matched.
///
/// The whole filter set and not just the org, because the count beside a pattern's name has
/// to be a count of the calls on screen. The list itself is never filtered: every pattern
/// the org has is in it, and one this window holds nothing for reports 0 rather than
/// vanishing — a pattern that disappears when you narrow the range looks deleted.
async fn list_patterns(
    State(app): State<App>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let filters = filters(query.as_deref())?;
    let org = filters.org.ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "a pattern belongs to an org, so ?org= is required",
        )
    })?;
    let db = app.db();
    known_org(&db, org)?;
    let counts = queries::pattern_counts(&db, &filters)?;
    let rows: Vec<Value> = db
        .list_patterns(org)?
        .iter()
        .map(|p| pattern_json(p, Some(counts.get(&p.id).copied().unwrap_or(0))))
        .collect();
    Ok(Json(rows).into_response())
}

/// One pattern as the browser reads it. The four JSON columns are parsed on the way out;
/// one that will not parse comes back as `null` rather than taking the row with it.
///
/// `matched` is a count *of a selection*, so it is `None` — and null on the wire — wherever
/// there is no selection to count. A pattern that was just edited has not been counted
/// against anything, and saying 0 there would be a number the analyst could believe.
fn pattern_json(p: &crate::db::Pattern, matched: Option<i64>) -> Value {
    let parse = |raw: &Option<String>| -> Value {
        raw.as_deref()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .unwrap_or(Value::Null)
    };
    json!({
        "id": p.id,
        "org_id": p.org_id,
        "matched": matched,
        "name": p.name,
        "criterion": p.criterion,
        "assistant_ids": parse(&p.assistant_ids),
        "plan": parse(&p.plan),
        "rule": parse(&p.rule),
        "chart": parse(&p.chart),
        "model": p.model,
        "mode": p.mode,
        "daily_cap_usd": p.daily_cap_usd,
        "sample_size": p.sample_size,
        "agreement": p.agreement,
        "created_at": p.created_at,
    })
}

/// The three modes of D-8. `free` costs nothing; the other two put a model in the loop and
/// are the reason the cap below is required rather than optional.
const MODES: [&str; 3] = ["free", "hybrid", "full"];

/// What the analyst owns on a saved pattern: what it matches, whether a model helps, and
/// how much that model may spend in a day. All three are sent every time, so "leave this
/// alone" and "clear this" are not the same request.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatternEdit {
    /// The rule, as the DSL spells it. `null` clears it, which is what a pattern being
    /// re-learned looks like between the two halves of the wizard.
    rule: Option<Value>,
    mode: String,
    daily_cap_usd: f64,
}

async fn update_pattern(
    State(app): State<App>,
    Path(id): Path<i64>,
    Json(body): Json<PatternEdit>,
) -> Result<Response, ApiError> {
    if !MODES.contains(&body.mode.as_str()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("mode must be one of {}", MODES.join(", ")),
        ));
    }
    if !(body.daily_cap_usd > 0.0 && body.daily_cap_usd.is_finite()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "daily_cap_usd must be a positive number of dollars",
        ));
    }
    // Checked here, in the engine, and refused while the analyst is looking at it. A rule
    // stored unchecked is a rule that fails at the next unattended `apply`, naming a
    // pattern nobody is in front of any more.
    let rule = match &body.rule {
        Some(Value::Null) | None => None,
        Some(value) => {
            let text = serde_json::to_string(value).map_err(anyhow::Error::from)?;
            rules::validate(&text, &format!("pattern {id}"))
                .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
            Some(text)
        }
    };

    let db = app.db();
    known_pattern(&db, id)?;
    db.set_pattern_rule(id, rule.as_deref(), &body.mode, body.daily_cap_usd)?;
    // The stored row, so the caller never has to assume what landed.
    Ok(Json(db.pattern(id)?.as_ref().map(|p| pattern_json(p, None))).into_response())
}

/// Re-run one pattern's rule over its org's calls. Costs nothing in any mode: this is the
/// rule half, and the rule half is arithmetic.
async fn apply_pattern(State(app): State<App>, Path(id): Path<i64>) -> Result<Response, ApiError> {
    let mut db = app.db();
    known_pattern(&db, id)?;
    // A rule this refuses is very nearly always the reason it failed, and `validate` names
    // the key it choked on. A 500 would tell the analyst it was the server's fault.
    let applied = rules::apply_one(&mut db, id)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    match applied {
        Some(applied) => {
            Ok(Json(json!({ "matched": applied.matched, "of": applied.of })).into_response())
        }
        None => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("pattern {id} has no rule to run yet"),
        )),
    }
}

fn known_pattern(db: &Db, id: i64) -> Result<(), ApiError> {
    match db.pattern(id)? {
        Some(_) => Ok(()),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no pattern {id}"),
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
