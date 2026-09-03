//! The optional password gate. Off unless `GRAPHIFY_PASSWORD` is set, because graphify
//! binds to loopback and the common case is one person on their own machine. Set it and
//! every `/api/*` route but the login itself needs a session cookie.
//!
//! Sessions live in memory only. A restart logs everyone out, which for a dashboard is
//! the right trade: no session table to leak, and nothing to expire on disk.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine as _;
use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Mutex;

/// The cookie the browser sends back. `HttpOnly` keeps it away from page scripts and
/// `SameSite=Strict` keeps another site from riding it.
pub const COOKIE: &str = "graphify_session";

const TOKEN_BYTES: usize = 32;

pub struct Auth {
    /// `None` means no password was configured and every request is allowed through.
    password: Option<String>,
    sessions: Mutex<HashSet<String>>,
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
            sessions: Mutex::new(HashSet::new()),
        }
    }

    pub fn required(&self) -> bool {
        self.password.is_some()
    }

    /// A new session token if the password matches, `None` if it does not.
    pub fn login(&self, given: &str) -> Option<String> {
        let expected = self.password.as_deref()?;
        if !same(given, expected) {
            return None;
        }
        let token = new_token();
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.clone());
        Some(token)
    }

    /// Whether a request carrying this `Cookie` header may proceed. With no password
    /// configured everything may.
    pub fn allows(&self, cookie_header: Option<&str>) -> bool {
        if !self.required() {
            return true;
        }
        let Some(token) = cookie_header.and_then(session_cookie) else {
            return false;
        };
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&token)
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

/// The `Set-Cookie` value for a fresh session. No `Max-Age`: the session dies with the
/// browser or with the process, whichever comes first.
pub fn set_cookie(token: &str) -> String {
    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict")
}
