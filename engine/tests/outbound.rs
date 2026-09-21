//! The first **Must never**, kept over the program instead of over one file: nothing leaves
//! this process for a provider except what that provider's kind is allowed to send.
//!
//! Two kinds, because the rule was always about two different risks (A-1, approved
//! 2026-09-21; `docs/prd-jev.md` §8). A **data connector** reads a client's live account —
//! `vapi.rs` is one — and is GET only, forever, no exceptions: that is what stops graphify
//! ever mutating a customer's org. A **decision connector** holds no graphify data and owns
//! nothing we could damage; it may POST, from one named file, and may still name no other
//! verb. Everything else may not reach out at all.
//!
//! Three guards that fail for different reasons. The first reads the source tree and
//! decides which files may reach out and how — a file nobody has heard of cannot inherit a
//! rule, so the lists below *are* the inheritance the spec promises. The second hands
//! made-up file bodies to the same rules, because a rule that has only ever been run
//! against a complying tree has never been seen to fail. The third watches a mock server
//! that accepts every method and asserts what actually left. Text cannot see a `Method`
//! held in a variable; a wire test cannot see a function nothing calls; neither can see a
//! rule that is simply wrong. Each guard covers the others' blind spot.

use graphify::vapi::{fetch_all_at, fetch_calls_at, FetchOpts, Retry};
use serde_json::json;
use std::path::{Path, PathBuf};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The files allowed to read a client's live account, as paths under `engine/src`. Putting
/// a file here is agreeing it is **GET only, forever, no exceptions**; `faults` then holds
/// it to that. This list does not move: it is the one that protects somebody else's data.
///
/// Paths, not file names: `src/vapi.rs` is the connector, and a `src/anywhere/vapi.rs`
/// would not inherit its pass by being called the same thing.
const DATA_CONNECTORS: [&str; 1] = ["vapi.rs"];

/// The files allowed to ask an outside model for a judgement. A decision provider holds
/// none of our data and owns nothing we could damage, so it may POST — and nothing else.
///
/// One file: `jev.rs`, added by S-73. `only_one_file_may_ever_post` keeps it at one, because
/// A-1 grants POST to a named file, not to a category that grows by a line. Nothing here
/// checks that the provider behind a name really is data-free; that is carried by the review
/// which adds the name, and by nothing else.
const DECISION_CONNECTORS: [&str; 1] = ["jev.rs"];

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

/// The whole of what a decision connector gets that a data connector does not: one verb,
/// in one spelling. `Method::` stays refused on purpose — a POST built through a variable
/// is a POST this file's text guard cannot see, so there is exactly one way to write one.
const DECISION_MAY: [&str; 1] = [".post("];

/// What a file is allowed to do, decided by which list names it — and `Ordinary`, which is
/// every file that is on neither and may not reach out at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Data,
    Decision,
    Ordinary,
}

fn role(file: &str) -> Role {
    if DATA_CONNECTORS.contains(&file) {
        Role::Data
    } else if DECISION_CONNECTORS.contains(&file) {
        Role::Decision
    } else {
        Role::Ordinary
    }
}

/// Every rule this file body breaks, one sentence each. Empty means allowed.
///
/// The rules live here, as a function over a role and some text, rather than inline in the
/// tests that walk the tree. That is the only reason they can be *tested*: the real tree
/// complies, so a rule read against it alone can be wrong in either direction — a missing
/// verb, an inverted branch — and stay green forever.
fn faults(role: Role, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    match role {
        Role::Ordinary => {
            for client in CLIENTS {
                if text.contains(client) {
                    out.push(format!(
                        "names an HTTP client ({client}) and is on neither connector list. \
                         Add it to DATA_CONNECTORS (GET only, forever) if it reads a \
                         client's account, or to DECISION_CONNECTORS if it asks an outside \
                         model a question. That list is how a file inherits the rule."
                    ));
                }
            }
        }
        Role::Data | Role::Decision => {
            for verb in NOT_A_GET {
                if role == Role::Decision && DECISION_MAY.contains(&verb) {
                    continue;
                }
                if text.contains(verb) {
                    out.push(match role {
                        Role::Data => format!(
                            "is a data connector and can send a {verb}. A data connector \
                             reads a client's live account: GET only, forever."
                        ),
                        _ => format!(
                            "is a decision connector and can send a {verb}. A decision \
                             connector may POST and nothing else, spelled `.post(`."
                        ),
                    });
                }
            }
            if !CLIENTS.iter().any(|c| text.contains(c)) {
                out.push(
                    "is on a connector list and makes no requests any more; take it off \
                     the list rather than leave a name guarding nothing"
                        .to_string(),
                );
            }
        }
    }
    out
}

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

/// A source file's path relative to `engine/src`, which is how the lists name them and how
/// a failure reports them: `net/mod.rs`, not a second file called `mod.rs`.
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
/// being a connector; it gets it by being named on a list, and until it is named it cannot
/// make a request at all.
#[test]
fn only_a_named_connector_reaches_out() {
    for path in sources() {
        let file = name(&path);
        if role(&file) != Role::Ordinary {
            continue;
        }
        let broken = faults(Role::Ordinary, &std::fs::read_to_string(&path).unwrap());
        assert!(broken.is_empty(), "{file} {}", broken.join("; "));
    }
}

/// And what being named costs, which is now two different prices. Both halves matter: a
/// connector that sends more than its kind allows is the failure the rule is about, and a
/// connector that no longer sends anything is a name on a list guarding nothing.
#[test]
fn a_connector_sends_only_what_its_role_allows() {
    for file in DATA_CONNECTORS.into_iter().chain(DECISION_CONNECTORS) {
        let text = std::fs::read_to_string(src().join(file))
            .unwrap_or_else(|_| panic!("{file} is on a connector list but is not in engine/src"));
        let broken = faults(role(file), &text);
        assert!(broken.is_empty(), "{file} {}", broken.join("; "));
    }
}

/// A file body that names a client and sends nothing but a GET — the shape a connector of
/// either kind has to have before any of the rules below mean anything.
const GETS: &str = "let c = reqwest::Client::new(); c.get(url).send().await";

/// The rule that protects somebody else's data, seen failing. This is the one that does not
/// move: whatever a decision connector is allowed, a data connector is not.
#[test]
fn a_data_connector_may_not_post() {
    assert!(faults(Role::Data, GETS).is_empty(), "a GET-only data connector is allowed");

    let broken = faults(Role::Data, &format!("{GETS} c.post(url)"));
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert!(broken[0].contains("GET only, forever"), "{broken:?}");
}

/// The new half of the rule, and the reason it is one verb rather than a free hand.
#[test]
fn a_decision_connector_may_post_and_nothing_else() {
    let posts = format!("{GETS} c.post(url)");
    assert!(faults(Role::Decision, &posts).is_empty(), "a decision connector may POST");

    for verb in [".delete(", ".put(", ".request(", "Method::POST"] {
        let broken = faults(Role::Decision, &format!("{posts} {verb}"));
        assert_eq!(broken.len(), 1, "{verb}: {broken:?}");
        assert!(broken[0].contains("may POST and nothing else"), "{verb}: {broken:?}");
    }
}

/// The default, which is the rule for every file in the tree but one.
#[test]
fn an_ordinary_file_may_not_reach_out_at_all() {
    assert!(faults(Role::Ordinary, "fn add(a: u32) -> u32 { a + 1 }").is_empty());

    let broken = faults(Role::Ordinary, GETS);
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert!(broken[0].contains("neither connector list"), "{broken:?}");
}

/// A name on a list that guards nothing reads as a pass. Both kinds are held to this, or a
/// list slowly fills with files that stopped being connectors and kept their permission.
#[test]
fn a_connector_that_no_longer_reaches_out_is_a_name_guarding_nothing() {
    for kind in [Role::Data, Role::Decision] {
        let broken = faults(kind, "fn parse(s: &str) -> u32 { s.len() as u32 }");
        assert_eq!(broken.len(), 1, "{kind:?}: {broken:?}");
        assert!(broken[0].contains("take it off"), "{kind:?}: {broken:?}");
    }
}

/// Every rule broken is reported, not the first one. A guard that stops at the first fault
/// makes a two-problem file look like a one-problem file the moment the first is fixed.
#[test]
fn a_file_that_breaks_two_rules_reports_both() {
    let broken = faults(Role::Data, &format!("{GETS} c.post(url); c.delete(url)"));
    assert_eq!(broken.len(), 2, "{broken:?}");
}

/// The lists themselves, because a mistake here is silent: a file on both would be a data
/// connector that may POST, and nothing above would say so.
#[test]
fn a_file_cannot_be_both_a_data_and_a_decision_connector() {
    for file in DATA_CONNECTORS {
        assert!(
            !DECISION_CONNECTORS.contains(&file),
            "{file} is on both lists, which makes a data connector that may POST"
        );
    }
}

/// A-1 grants POST to *one named file*. Growing that list is a spec amendment, not an edit.
#[test]
fn only_one_file_may_ever_post() {
    let named = DECISION_CONNECTORS.len();
    assert!(
        named <= 1,
        "A-1 grants POST to one named file, not to {named}: {DECISION_CONNECTORS:?}"
    );
}

/// What the text cannot spell. The mock accepts every method, so a POST is answered rather
/// than refused, and the assertion below is the only thing that fails — which is the point:
/// a guard that reds because a mock did not match tells you the mock did not match.
///
/// This drives the data connector, and every request it makes is a GET. The name says
/// *data connector* and not *everything* on purpose: S-73 made a POST expressible, and a
/// test whose name claims more than its body drives is the exact fault S-72 was written
/// about. The decision connector's own wire proof lives in `engine/tests/jev.rs`.
#[tokio::test]
async fn every_request_the_data_connector_makes_is_a_get() {
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
