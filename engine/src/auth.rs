//! The optional password gate. Off unless `GRAPHIFY_PASSWORD` is set, because graphify
//! binds to loopback and the common case is one person on their own machine. Set it and
//! every `/api/*` route but the login itself needs a session cookie.
//!
//! A session has two ends and they belong to different machines. The cookie carries no
//! `Max-Age`, so the browser drops it when it closes — that end is the browser's. The token
//! behind the cookie is held here, and this process lets go of it after `SESSION_TTL` or
//! when it stops. Only the second end is something the server does, which is why both have
//! to be said out loud: a cookie value that got out — copied off a shared machine, read out
//! of a proxy log — is a working credential for exactly as long as this side goes on
//! honouring it, and a browser closing somewhere is not this side letting go.
//!
//! Nothing is written down. A restart still signs everyone out, which for a dashboard is
//! the right trade: no session table to leak, and nothing to expire on disk.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine as _;
use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The cookie the browser sends back. `HttpOnly` keeps it away from page scripts and
/// `SameSite=Strict` keeps another site from riding it.
pub const COOKIE: &str = "graphify_session";

/// How long a session lasts, measured from the login that issued it. A day: the day is
/// already this product's unit — the sync, the spend cap and the schedule all run on one —
/// and signing in this morning should see you to the evening.
///
/// Absolute rather than sliding, which is the less usual choice and the deliberate one.
/// `Notices` polls from every screen every thirty seconds, so an idle timeout would be
/// renewed for as long as a tab was open anywhere; it would expire only sessions whose
/// browser had already closed, which is the case the cookie covers by itself.
pub const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

const TOKEN_BYTES: usize = 32;

pub struct Auth {
    /// `None` means no password was configured and every request is allowed through.
    password: Option<String>,
    /// Each token against the moment it was issued. Not a bare set: a session carrying no
    /// issue time is one nothing is able to decide to let go of. `Instant` and not a wall
    /// clock, so that moving the machine's clock cannot lengthen a session.
    sessions: Mutex<HashMap<String, Instant>>,
    ttl: Duration,
}

/// Holds a password, so it prints its name and nothing else.
impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Auth {{ required: {} }}", self.required())
    }
}

impl Auth {
    /// Read the password from the environment. Whitespace is not a password: an empty
    /// `GRAPHIFY_PASSWORD=` in a compose file must leave the gate open rather than lock
    /// everyone out with a value nobody can type.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var("GRAPHIFY_PASSWORD")
                .ok()
                .filter(|p| !p.trim().is_empty()),
        )
    }

    pub fn new(password: Option<String>) -> Self {
        Auth {
            password,
            sessions: Mutex::new(HashMap::new()),
            ttl: SESSION_TTL,
        }
    }

    /// The same gate with a shorter life, so that a test can outlive a session without
    /// waiting out a day. Nothing in the binary calls this.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn required(&self) -> bool {
        self.password.is_some()
    }

    /// A new session token if the password matches, `None` if it does not.
    ///
    /// Expired entries go here and not only on the request that presents one, because a
    /// session nobody comes back to is never presented again: without this the map would be
    /// bounded by how many times the process had ever been signed into rather than by how
    /// many sessions are live.
    pub fn login(&self, given: &str) -> Option<String> {
        let expected = self.password.as_deref()?;
        if !same(given, expected) {
            return None;
        }
        let token = new_token();
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, issued| issued.elapsed() < self.ttl);
        sessions.insert(token.clone(), Instant::now());
        Some(token)
    }

    /// Whether a request carrying this `Cookie` header may proceed. With no password
    /// configured everything may.
    ///
    /// An expired token is dropped rather than merely refused. Answering 401 while going on
    /// holding it would leave the server still carrying a credential it has decided is no
    /// longer good, which is the whole of what this is about.
    pub fn allows(&self, cookie_header: Option<&str>) -> bool {
        if !self.required() {
            return true;
        }
        let Some(token) = cookie_header.and_then(session_cookie) else {
            return false;
        };
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.get(&token) {
            Some(issued) if issued.elapsed() < self.ttl => true,
            Some(_) => {
                sessions.remove(&token);
                false
            }
            None => false,
        }
    }
}

/// Compare over SHA-256 digests rather than the strings: fixed width, so the comparison
/// cannot leak the password's length, and no early exit on the first differing byte.
fn same(given: &str, expected: &str) -> bool {
    let (a, b) = (
        Sha256::digest(given.as_bytes()),
        Sha256::digest(expected.as_bytes()),
    );
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    B64URL.encode(bytes)
}

/// Pull our cookie out of a `Cookie` header that may hold several.
fn session_cookie(header: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == COOKIE).then(|| value.trim().to_string())
    })
}

/// The `Set-Cookie` value for a fresh session. No `Max-Age`, and not as an oversight: a
/// `Max-Age` would make this a persistent cookie, so it would begin surviving the browser
/// close that today ends it. The session's own end is `SESSION_TTL`, kept on this side of
/// the wire where whoever holds the cookie cannot edit it.
pub fn set_cookie(token: &str) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Short enough that a test can outlive a session, long enough that a loaded machine
    /// getting to the sleep late does not expire one that should still be good.
    const BRIEF: Duration = Duration::from_millis(50);

    fn gate() -> Auth {
        Auth::new(Some("hunter2".into())).with_ttl(BRIEF)
    }

    fn cookie(token: &str) -> String {
        format!("{COOKIE}={token}")
    }

    fn held(auth: &Auth) -> usize {
        auth.sessions.lock().unwrap().len()
    }

    #[test]
    fn a_session_stops_working_once_it_is_older_than_the_ttl() {
        let auth = gate();
        let token = auth.login("hunter2").unwrap();
        assert!(auth.allows(Some(&cookie(&token))), "a fresh session was refused");

        std::thread::sleep(BRIEF * 2);

        assert!(
            !auth.allows(Some(&cookie(&token))),
            "an expired session was let through"
        );
    }

    /// Refusing it is half the job. A 401 that leaves the token on file is a map that still
    /// grows for the life of the process, and a credential the server has not let go of.
    #[test]
    fn an_expired_session_is_dropped_and_not_merely_refused() {
        let auth = gate();
        let token = auth.login("hunter2").unwrap();
        assert_eq!(held(&auth), 1);

        std::thread::sleep(BRIEF * 2);
        auth.allows(Some(&cookie(&token)));

        assert_eq!(held(&auth), 0, "the token was refused but kept");
    }

    /// A session nobody comes back to is never presented, so `allows` never gets the chance
    /// to drop it. Without a sweep somewhere, the only bound on the map is how many logins
    /// the process has served.
    #[test]
    fn a_login_clears_out_the_sessions_nobody_came_back_for() {
        let auth = gate();
        let abandoned = auth.login("hunter2").unwrap();
        for _ in 0..2 {
            auth.login("hunter2").unwrap();
        }
        assert_eq!(held(&auth), 3);

        std::thread::sleep(BRIEF * 2);
        let fresh = auth.login("hunter2").unwrap();

        assert_eq!(held(&auth), 1, "the abandoned sessions are still held");
        assert!(
            !auth.allows(Some(&cookie(&abandoned))),
            "a swept session still opened the gate"
        );
        assert!(
            auth.allows(Some(&cookie(&fresh))),
            "the new session was swept along with the old"
        );
    }

    /// With no password there is nothing to expire, and the TTL must not turn into a way
    /// for an unguarded install to lock itself out.
    #[test]
    fn an_open_gate_is_not_touched_by_the_ttl() {
        let auth = Auth::new(None).with_ttl(BRIEF);
        std::thread::sleep(BRIEF * 2);

        assert!(auth.allows(None));
        assert!(auth.allows(Some(&cookie("never issued"))));
        assert_eq!(held(&auth), 0, "an open gate kept a session");
    }

    /// The gate the binary actually builds. Every other test here hands itself a short TTL
    /// through `with_ttl`, so a `new` that quietly started issuing a longer-lived session
    /// would leave all of them green and nothing would be watching the one that ships.
    #[test]
    fn the_gate_the_binary_builds_expires_at_the_session_ttl() {
        assert_eq!(Auth::new(Some("hunter2".into())).ttl, SESSION_TTL);
        assert_eq!(Auth::from_env().ttl, SESSION_TTL);
    }

    /// The cookie's end is the browser's and the session's end is ours. Either attribute
    /// here would take the first one away by making the cookie persistent.
    #[test]
    fn the_cookie_does_not_outlive_the_browser() {
        let set = set_cookie("t").to_ascii_lowercase();

        assert!(!set.contains("max-age"), "was: {set}");
        assert!(!set.contains("expires"), "was: {set}");
    }
}
