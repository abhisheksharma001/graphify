use assert_cmd::Command;
use graphify::db::Db;
use tempfile::TempDir;

/// A database of its own for each test. Without it the subcommands would reach for
/// `data/graphify.db` in the repo, and two tests migrating the same new file at once is a
/// race as well as a mess.
fn db_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    Db::open(dir.path().join("graphify.db"))
        .unwrap()
        .create_org("acme")
        .unwrap();
    dir
}

fn graphify(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("graphify").unwrap();
    cmd.env("GRAPHIFY_DB", dir.path().join("graphify.db"))
        .env_remove("VAPI_API_KEY");
    cmd
}

#[test]
fn version_prints_name_and_version() {
    Command::cargo_bin("graphify")
        .unwrap()
        .arg("version")
        .assert()
        .success()
        .stdout("graphify 0.1.0\n");
}

#[test]
fn no_subcommand_is_an_error_with_usage() {
    let output = Command::cargo_bin("graphify").unwrap().assert().failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(stderr.contains("Usage:"), "stderr was: {stderr}");
}

/// With no key in the environment and none in the store, the error has to name the way
/// out that costs nothing to try.
#[test]
fn sync_without_a_key_says_which_variable_is_missing() {
    let dir = db_dir();
    let output = graphify(&dir)
        .args(["sync", "--org", "acme"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(stderr.contains("VAPI_API_KEY"), "stderr was: {stderr}");
}

/// Same ordering as `sync`.
#[test]
fn assistants_without_a_key_says_which_variable_is_missing() {
    let dir = db_dir();
    let output = graphify(&dir)
        .args(["assistants", "--org", "acme"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(stderr.contains("VAPI_API_KEY"), "stderr was: {stderr}");
}

/// The key now comes from the org's row, so an org that does not exist is the first
/// thing that goes wrong — before any key is looked for and long before any request.
#[test]
fn syncing_an_unknown_org_names_the_org() {
    let dir = db_dir();
    let output = graphify(&dir)
        .args(["sync", "--org", "globex"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(stderr.contains("globex"), "stderr was: {stderr}");
}

#[test]
fn serve_is_a_subcommand() {
    Command::cargo_bin("graphify")
        .unwrap()
        .args(["serve", "--help"])
        .assert()
        .success();
}

/// The flag exists and is spelled the way the docs say. A default that opens a browser
/// needs a way off it for anyone running this over ssh or in a container.
#[test]
fn serve_takes_no_open() {
    let output = Command::cargo_bin("graphify")
        .unwrap()
        .args(["serve", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).to_string();
    assert!(stdout.contains("--no-open"), "stdout was: {stdout}");
}

/// `--org all` is what a scheduled line uses, and the reason it exists is that a machine
/// syncing at six has nobody to name the orgs for it. One org without a key must not stop
/// the ones after it — and the run must still say it went wrong, or a cron log full of
/// zeroes reads as a quiet morning.
#[test]
fn syncing_all_orgs_keeps_going_past_a_failure_and_still_fails() {
    let dir = db_dir();
    Db::open(dir.path().join("graphify.db"))
        .unwrap()
        .create_org("globex")
        .unwrap();
    let output = graphify(&dir)
        .args(["sync", "--org", "all"])
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    for org in ["acme", "globex"] {
        assert!(stdout.contains(&format!("--- {org}")), "stdout was: {stdout}");
        assert!(stderr.contains(org), "stderr was: {stderr}");
    }
    assert!(stderr.contains("2 of 2 orgs failed"), "stderr was: {stderr}");
}

/// A fresh install has none, and that is not an error to wake anyone for.
#[test]
fn syncing_all_orgs_on_an_empty_database_says_so_and_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    Db::open(dir.path().join("graphify.db")).unwrap();
    let output = graphify(&dir)
        .args(["sync", "--org", "all"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).to_string();
    assert!(stdout.contains("no orgs yet"), "stdout was: {stdout}");
}
