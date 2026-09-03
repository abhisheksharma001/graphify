use graphify::db::Db;
use graphify::secrets::{Secrets, Status, NAMES};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

/// A distinctive value, so grepping the database file for it means something.
const PLAIN: &str = "sk-plaintext-must-never-land-0001";

/// `get` and `status` read the process environment, which every test shares. Anything
/// that touches an override takes this first so the tests do not race each other.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn clear_env() {
    for var in [
        "VAPI_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GRAPHIFY_SECRET",
    ] {
        std::env::remove_var(var);
    }
}

struct Fixture {
    _dir: TempDir,
    db_path: PathBuf,
    key_path: PathBuf,
    db: Db,
    secrets: Secrets,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("graphify.db");
    let key_path = dir.path().join(".secret");
    let db = Db::open(&db_path).unwrap();
    db.create_org("acme").unwrap();
    let secrets = Secrets::open(&key_path).unwrap();
    Fixture {
        _dir: dir,
        db_path,
        key_path,
        db,
        secrets,
    }
}

impl Fixture {
    fn db_bytes(&self) -> Vec<u8> {
        std::fs::read(&self.db_path).unwrap()
    }
}

/// The spec's acceptance case, both halves.
#[test]
fn a_stored_key_is_absent_from_the_database_file_and_shows_only_its_tail() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();

    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();

    let bytes = f.db_bytes();
    assert!(
        !bytes
            .windows(PLAIN.len())
            .any(|w| w == PLAIN.as_bytes()),
        "the plaintext key is sitting in the database file"
    );

    let status = f.secrets.status(&f.db, 1).unwrap();
    assert_eq!(
        status[0],
        Status {
            name: "vapi".into(),
            set: true,
            last4: Some("0001".into()),
        }
    );
}

#[test]
fn a_value_comes_back_out_unchanged() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();

    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();

    assert_eq!(
        f.secrets.get(&f.db, 1, "vapi").unwrap().unwrap().expose(),
        PLAIN
    );
}

#[test]
fn an_unset_secret_is_none_and_reports_unset() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();

    assert!(f.secrets.get(&f.db, 1, "vapi").unwrap().is_none());
    let status = f.secrets.status(&f.db, 1).unwrap();
    assert_eq!(status.len(), NAMES.len());
    assert!(status.iter().all(|s| !s.set && s.last4.is_none()));
}

#[test]
fn setting_a_name_twice_replaces_it() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();

    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();
    f.secrets.set(&f.db, 1, "vapi", "sk-the-second-one-9999").unwrap();

    assert_eq!(
        f.secrets.get(&f.db, 1, "vapi").unwrap().unwrap().expose(),
        "sk-the-second-one-9999"
    );
    assert_eq!(f.secrets.status(&f.db, 1).unwrap()[0].last4.as_deref(), Some("9999"));
}

/// The environment wins, and a variable set to nothing is not an override — an empty
/// `VAPI_API_KEY=` in a compose file must not mask a perfectly good stored key.
#[test]
fn the_environment_overrides_the_store_unless_it_is_empty() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();
    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();

    std::env::set_var("VAPI_API_KEY", "sk-from-the-environment-7777");
    assert_eq!(
        f.secrets.get(&f.db, 1, "vapi").unwrap().unwrap().expose(),
        "sk-from-the-environment-7777"
    );
    assert_eq!(f.secrets.status(&f.db, 1).unwrap()[0].last4.as_deref(), Some("7777"));

    std::env::set_var("VAPI_API_KEY", "   ");
    assert_eq!(
        f.secrets.get(&f.db, 1, "vapi").unwrap().unwrap().expose(),
        PLAIN,
        "an empty variable must fall through to the store"
    );

    clear_env();
    assert_eq!(
        f.secrets.get(&f.db, 1, "vapi").unwrap().unwrap().expose(),
        PLAIN
    );
}

/// A key supplied only by the environment is still a key the org has.
#[test]
fn an_environment_only_secret_reads_as_set() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();

    std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-env-only-4242");
    let status = f.secrets.status(&f.db, 1).unwrap();
    clear_env();

    let anthropic = status.iter().find(|s| s.name == "anthropic").unwrap();
    assert!(anthropic.set);
    assert_eq!(anthropic.last4.as_deref(), Some("4242"));
}

/// The whole point of the wrapper: there is no formatting that prints the value.
#[test]
fn a_secret_never_formats_as_its_value() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();
    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();

    let secret = f.secrets.get(&f.db, 1, "vapi").unwrap().unwrap();

    assert_eq!(format!("{secret:?}"), "***");
    assert_eq!(format!("{secret}"), "***");
}

/// The ciphertext is bound to its row, so lifting one into another name or another org
/// fails instead of handing back the wrong key.
#[test]
fn a_ciphertext_moved_to_another_row_will_not_decrypt() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();
    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();
    let (blob, tail) = f.db.secret(1, "vapi").unwrap().unwrap();

    f.db.upsert_secret(1, "anthropic", &blob, tail.as_deref(), "now").unwrap();
    f.db.upsert_secret(2, "vapi", &blob, tail.as_deref(), "now").unwrap();

    for (org, name) in [(1, "anthropic"), (2, "vapi")] {
        let err = f.secrets.get(&f.db, org, name).unwrap_err();
        assert!(err.to_string().contains("could not be decrypted"), "was: {err}");
        assert!(!err.to_string().contains(PLAIN));
    }
}

#[test]
fn a_different_key_cannot_read_the_store() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();
    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();

    let other_dir = tempfile::tempdir().unwrap();
    let other = Secrets::open(other_dir.path().join(".secret")).unwrap();

    let err = other.get(&f.db, 1, "vapi").unwrap_err();
    assert!(err.to_string().contains("could not be decrypted"), "was: {err}");
}

/// A key file must never exist readable, not even for the instant before a chmod.
#[test]
fn the_key_file_is_created_private_and_reused() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();

    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(&f.key_path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);

    // Re-opening must reuse the same key, or every restart would orphan every secret.
    f.secrets.set(&f.db, 1, "vapi", PLAIN).unwrap();
    let again = Secrets::open(&f.key_path).unwrap();
    assert_eq!(
        again.get(&f.db, 1, "vapi").unwrap().unwrap().expose(),
        PLAIN
    );
}

#[test]
fn a_graphify_secret_of_the_wrong_length_is_refused() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();

    std::env::set_var("GRAPHIFY_SECRET", "dG9vLXNob3J0");
    let err = Secrets::open(dir.path().join(".secret")).unwrap_err();
    std::env::remove_var("GRAPHIFY_SECRET");

    assert!(format!("{err:#}").contains("32 bytes"), "was: {err:#}");
    assert!(
        !dir.path().join(".secret").exists(),
        "a refused key must not leave a file behind"
    );
}

/// Four characters of a short secret is most of the secret.
#[test]
fn a_short_value_gets_no_tail() {
    let _guard = env_lock();
    clear_env();
    let f = fixture();

    f.secrets.set(&f.db, 1, "vapi", "abc123").unwrap();

    let status = f.secrets.status(&f.db, 1).unwrap();
    assert!(status[0].set);
    assert_eq!(status[0].last4, None);
}

/// The documented primary path: an operator supplies the key, and nothing is written to
/// disk for an attacker to find.
#[test]
fn graphify_secret_supplies_the_key_and_writes_no_file() {
    let _guard = env_lock();
    clear_env();
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    db.create_org("acme").unwrap();
    let key_path = dir.path().join(".secret");

    // 32 zero bytes, base64. A real deployment generates its own.
    std::env::set_var("GRAPHIFY_SECRET", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    let secrets = Secrets::open(&key_path).unwrap();
    secrets.set(&db, 1, "vapi", PLAIN).unwrap();
    let reopened = Secrets::open(&key_path).unwrap();
    let got = reopened.get(&db, 1, "vapi").unwrap().unwrap().expose().to_string();
    clear_env();

    assert_eq!(got, PLAIN);
    assert!(!key_path.exists(), "the env key must not be written to disk");
}
