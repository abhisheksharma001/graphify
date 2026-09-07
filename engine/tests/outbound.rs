//! The first **Must never**, kept over the program instead of over one file: nothing but a
//! GET leaves this process for a provider.
//!
//! Two guards that fail for different reasons. The first reads the source tree and decides
//! which files may reach out at all — a file nobody has heard of cannot inherit a rule, so
//! the list below *is* the inheritance the spec promises. The second watches a mock server
//! that accepts every method and asserts what actually left. Text cannot see a `Method`
//! held in a variable; a wire test cannot see a function nothing calls. Each guard covers
//! the other's blind spot, which is why there are two.

use graphify::vapi::{fetch_all_at, fetch_calls_at, FetchOpts, Retry};
use serde_json::json;
use std::path::{Path, PathBuf};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The files allowed to make an outbound request, as paths under `engine/src`. Putting a
/// file here is agreeing it is GET-only forever; `a_connector_sends_nothing_but_a_get` then
/// holds it to that. Everything else in the tree may not reach out at all.
///
/// Paths, not file names: `src/vapi.rs` is the connector, and a `src/anywhere/vapi.rs`
/// would not inherit its pass by being called the same thing.
const CONNECTORS: [&str; 1] = ["vapi.rs"];

/// How an HTTP client gets named. `reqwest` is the one in `Cargo.toml`; the rest are here
/// so that reaching for a different crate is the same failure rather than a way around it.
const CLIENTS: [&str; 5] = ["reqwest", "hyper::Client", "ureq", "curl::", "isahc"];

/// Every way of sending something that is not a plain GET, including the reflective ones —
/// `http.request(Method::POST, url)` and `http.execute(..)` contain none of the four verbs.
/// This list can afford to be broad because it is only ever read against a connector file:
/// `server.rs` routes `.post(create_org)`, and `server.rs` is not a connector.
const NOT_A_GET: [&str; 8] = [
    ".post(",
    ".put(",
    ".patch(",
    ".delete(",
    ".head(",
    ".request(",
    ".execute(",
    "Method::",
];

/// Every `.rs` file under `engine/src`, found by walking rather than by listing, so a file
/// added tomorrow is in scope the moment it exists and a new subdirectory is not a hole.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("engine/src is readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&src(), &mut out);
    out.sort();
    out
}

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// A source file's path relative to `engine/src`, which is how `CONNECTORS` names them and
/// how a failure reports them: `net/mod.rs`, not a second file called `mod.rs`.
fn name(path: &Path) -> String {
    path.strip_prefix(src())
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// A harvest that finds nothing passes everything. This is the guard on the guard: the walk
/// has to reach the tree it claims to cover, including the files the rule is about.
#[test]
fn the_walk_reaches_the_source_tree() {
    let found: Vec<String> = sources().iter().map(|p| name(p)).collect();

    assert!(
        found.len() >= 15,
        "the walk found {} files, so it is not reading engine/src: {found:?}",
        found.len()
    );
    for expected in ["vapi.rs", "server.rs", "db.rs", "lib.rs"] {
        assert!(found.iter().any(|f| f == expected), "the walk missed {expected}");
    }
}

/// The inheritance the Must-never describes. A second connector does not get the rule by
/// being a connector; it gets it by being named here, and until it is named it cannot make
/// a request at all.
#[test]
fn only_a_named_connector_reaches_out() {
    for path in sources() {
        let file = name(&path);
        if CONNECTORS.contains(&file.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for client in CLIENTS {
            assert!(
                !text.contains(client),
                "{file} names an HTTP client ({client}) and is not a connector. Add it to \
                 CONNECTORS in this file, which is how it inherits the GET-only rule."
            );
        }
    }
}

/// And what being named costs. Both halves matter: a connector that sends a write is the
/// failure the rule is about, and a connector that no longer sends anything is a name on a
/// list guarding nothing — take it off rather than leave it passing.
#[test]
fn a_connector_sends_nothing_but_a_get() {
    for file in CONNECTORS {
        let path = src().join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{file} is on CONNECTORS but is not in engine/src"));

        for verb in NOT_A_GET {
            assert!(!text.contains(verb), "connector {file} can send a {verb}");
        }
        assert!(
            CLIENTS.iter().any(|c| text.contains(c)),
            "{file} makes no requests any more; take it off CONNECTORS"
        );
    }
}

/// What the text cannot spell. The mock accepts every method, so a POST is answered rather
/// than refused, and the assertion below is the only thing that fails — which is the point:
/// a guard that reds because a mock did not match tells you the mock did not match.
#[tokio::test]
async fn every_request_that_leaves_is_a_get() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let retry = Retry { max: 0, base_ms: 0 };
    let opts = FetchOpts {
        last: 10,
        ..FetchOpts::default()
    };
    fetch_calls_at(&server.uri(), "k", &opts, retry).await.unwrap();
    fetch_all_at(&server.uri(), "k", "tool", retry).await.unwrap();

    let sent = server.received_requests().await.unwrap();
    assert!(!sent.is_empty(), "nothing left the process, so nothing was checked");
    for req in &sent {
        assert_eq!(
            req.method.as_str(),
            "GET",
            "a {} left for {}",
            req.method,
            req.url.path()
        );
    }
}
