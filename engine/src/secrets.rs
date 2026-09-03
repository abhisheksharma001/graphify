//! Secrets at rest: ChaCha20-Poly1305 over the `secrets` table, with the process
//! environment allowed to override any of them.
//!
//! Three rules shape this file. A plaintext key never reaches the database — only a
//! nonce, a ciphertext and the last four characters. A value that has been read out is a
//! [`Secret`], which prints as `***` no matter how it is formatted, so the only way to
//! see it is to call `expose` and the only way to leak it is to write that word. And the
//! ciphertext is bound to the row it belongs to, so moving one between orgs or names
//! fails to decrypt instead of quietly succeeding.

use crate::db::Db;
use crate::now;
use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use std::fmt;
use std::path::{Path, PathBuf};

/// The secrets graphify knows how to hold, in the order `status` reports them.
pub const NAMES: [&str; 3] = ["vapi", "anthropic", "openai"];

const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;

/// Shortest value that still gets a `last4`. Four characters of a four-character secret
/// is the whole secret.
const LAST4_MIN: usize = 8;

/// A secret value out of the store. Formatting it — `{}`, `{:?}`, a panic message, a
/// `dbg!` — prints `***`. `expose` is the only way through, and is the one word to grep
/// for when asking whether a key can reach a log.
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// What the settings screen may know about a secret: that it exists, and its tail.
#[derive(Debug, PartialEq, Eq)]
pub struct Status {
    pub name: String,
    pub set: bool,
    pub last4: Option<String>,
}

pub struct Secrets {
    cipher: ChaCha20Poly1305,
}

/// The cipher holds the key material, so this prints a name and nothing else.
impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secrets { key: *** }")
    }
}

/// Where the file key lives when the caller does not say: beside the database, so a
/// `GRAPHIFY_DB` pointing somewhere else takes its key with it.
pub fn default_key_path() -> PathBuf {
    crate::db::default_path().with_file_name(".secret")
}

impl Secrets {
    /// Take the key from `GRAPHIFY_SECRET` if it is set, else from `path`, creating it
    /// with mode 0600 if it is not there yet.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let key = match std::env::var("GRAPHIFY_SECRET") {
            Ok(v) if !v.trim().is_empty() => decode_key(v.trim())
                .context("GRAPHIFY_SECRET must be 32 bytes, base64-encoded")?,
            _ => file_key(path.as_ref())?,
        };
        Ok(Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(&key)),
        })
    }

    pub fn set(&self, db: &Db, org_id: i64, name: &str, value: &str) -> Result<()> {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let sealed = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: value.as_bytes(),
                    aad: aad(org_id, name).as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("encrypting the {name} secret failed"))?;

        // The nonce is not secret and is useless without the key, so it rides in front of
        // the ciphertext rather than needing a column of its own.
        let mut blob = nonce.to_vec();
        blob.extend(sealed);
        db.upsert_secret(org_id, name, &blob, last4(value).as_deref(), &now())
    }

    /// The value, from the environment if it is there and from the store otherwise.
    ///
    /// The environment wins so an operator can override a stored key without touching the
    /// database — the same order the CLI already used before there was a store.
    pub fn get(&self, db: &Db, org_id: i64, name: &str) -> Result<Option<Secret>> {
        if let Some(v) = from_env(name) {
            return Ok(Some(Secret(v)));
        }
        let Some((blob, _)) = db.secret(org_id, name)? else {
            return Ok(None);
        };
        if blob.len() <= NONCE_BYTES {
            bail!("the stored {name} secret is truncated");
        }
        let (nonce, sealed) = blob.split_at(NONCE_BYTES);
        let plain = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: sealed,
                    aad: aad(org_id, name).as_bytes(),
                },
            )
            // Wrong key, edited row, or a ciphertext lifted from another row. None of
            // those are worth telling apart, and the value cannot appear in the message.
            .map_err(|_| anyhow::anyhow!("the stored {name} secret could not be decrypted"))?;
        Ok(Some(Secret(
            String::from_utf8(plain).context("the stored secret is not UTF-8")?,
        )))
    }

    /// One row per known name, whether or not it is set. Reports what `get` would return,
    /// so a key supplied only by the environment still reads as set.
    pub fn status(&self, db: &Db, org_id: i64) -> Result<Vec<Status>> {
        NAMES
            .iter()
            .map(|name| {
                let (set, last4) = match from_env(name) {
                    Some(v) => (true, last4(&v)),
                    None => match db.secret(org_id, name)? {
                        Some((_, tail)) => (true, tail),
                        None => (false, None),
                    },
                };
                Ok(Status {
                    name: (*name).to_string(),
                    set,
                    last4,
                })
            })
            .collect()
    }
}

/// The environment variable that overrides a given secret, for the names that have one.
fn env_var(name: &str) -> Option<&'static str> {
    match name {
        "vapi" => Some("VAPI_API_KEY"),
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "openai" => Some("OPENAI_API_KEY"),
        _ => None,
    }
}

/// A variable that is set to whitespace is not an override. An empty `VAPI_API_KEY=` in a
/// compose file would otherwise mask a perfectly good stored key with nothing.
fn from_env(name: &str) -> Option<String> {
    let v = std::env::var(env_var(name)?).ok()?;
    (!v.trim().is_empty()).then(|| v.trim().to_string())
}

/// Binds a ciphertext to its row. Decryption of a blob moved to another org or name fails
/// rather than returning the wrong org's key.
fn aad(org_id: i64, name: &str) -> String {
    format!("{org_id}:{name}")
}

/// The tail shown in the UI, or nothing at all for a value short enough that four
/// characters would give most of it away.
fn last4(value: &str) -> Option<String> {
    (value.chars().count() >= LAST4_MIN)
        .then(|| value.chars().rev().take(4).collect::<Vec<_>>())
        .map(|mut c| {
            c.reverse();
            c.into_iter().collect()
        })
}

fn decode_key(b64: &str) -> Result<Vec<u8>> {
    let key = B64.decode(b64).context("not valid base64")?;
    if key.len() != KEY_BYTES {
        bail!("expected {KEY_BYTES} bytes, got {}", key.len());
    }
    Ok(key)
}

/// Read the key file, or make one. Unix only, deliberately: the mode is the protection,
/// and graphify ships in a Linux container.
fn file_key(path: &Path) -> Result<Vec<u8>> {
    if path.exists() {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        return decode_key(raw.trim()).with_context(|| format!("{} is corrupt", path.display()));
    }

    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
    }

    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let key = ChaCha20Poly1305::generate_key(&mut OsRng);
    // 0600 at creation, not chmod afterwards: a key must never exist readable, even for
    // the instant between the two calls.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?
        .write_all(B64.encode(key).as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(key.to_vec())
}
