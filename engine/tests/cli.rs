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

// S-53: a flag is a promise, and the dispatch is where promises are kept or discarded.
// `--print` says "write nothing" and was destructured into `_`, so `--print --install` was
// byte-identical to `--install`: the crontab line was never printed and the operator was
// asked to write a plist into their real home directory instead.

/// Every subcommand the CLI has, and every long flag on it, as clap renders them. Harvested
/// from the binary's own `--help` rather than read off `cli.rs`, so the list below is
/// checked against the parser that will actually run and not against the source it was
/// written from.
///
/// A help line starts with its flags and then stops being about flags: `-h, --help  Print
/// help`, `--org <ORG>  Org to sync…`. So the words are taken while they still look like
/// flags rather than one per line, because a long flag that grows a short alias is the
/// second word and would otherwise leave the harvest without a sound.
fn long_flags(subcommand: &str) -> Vec<String> {
    let output = Command::cargo_bin("graphify")
        .unwrap()
        .args([subcommand, "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).to_string();
    let mut flags: Vec<String> = stdout
        .lines()
        .flat_map(|line| line.split_whitespace().take_while(|word| word.starts_with('-')))
        .filter(|word| word.starts_with("--"))
        .filter(|word| *word != "--help")
        .map(|word| word.trim_end_matches(',').to_string())
        .collect();
    flags.sort();
    flags.dedup();
    flags
}

/// The whole argument surface, written down once. A flag added, renamed or removed makes
/// this red, which is the moment somebody has to say what the flag does — the step this
/// test comes from is one where nobody did. What it proves is that the surface is written
/// down, not that any flag on it is honoured: that is each flag's own test, and the two
/// below are `schedule`'s.
#[test]
fn every_subcommand_and_flag_is_one_somebody_wrote_down() {
    let surface: [(&str, &[&str]); 7] = [
        ("version", &[]),
        ("sync", &["--last", "--org", "--since"]),
        ("assistants", &["--org"]),
        ("serve", &["--no-open"]),
        ("apply", &[]),
        ("rule-check", &["--calls", "--rule"]),
        ("schedule", &["--at", "--install", "--org", "--print"]),
    ];
    for (subcommand, expected) in surface {
        assert_eq!(
            long_flags(subcommand),
            expected,
            "the flags on `{subcommand}` are not the ones written down here"
        );
    }
}

/// Two flags naming opposite intentions is a question for whoever typed them, not one to
/// answer for them by preferring the destructive one. Refused at parse time, so no branch
/// has to be trusted to get the order right, and refused *by name* so the message says
/// which two words cannot go together.
#[test]
fn print_and_install_cannot_both_be_asked_for() {
    let output = Command::cargo_bin("graphify")
        .unwrap()
        .args(["schedule", "--print", "--install"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    for flag in ["--print", "--install"] {
        assert!(stderr.contains(flag), "stderr was: {stderr}");
    }
    assert_eq!(
        output.get_output().status.code(),
        Some(2),
        "a refused pair is a usage error, stderr was: {stderr}"
    );
}

/// What the shipped binary could not do: tell the two flags apart. `--print --install` was
/// `--install` to the byte, and the only reason that read as harmless is that `--print`
/// alone and no flag at all produce the same output — so sameness had to be measured
/// against `--install`, which is the one that writes. Stdin is closed: `confirm` reads end
/// of input as no, so `--install` here asks and stops.
#[test]
fn print_and_install_do_not_do_the_same_thing() {
    let dir = db_dir();
    let printed = graphify(&dir)
        .args(["schedule", "--print"])
        .write_stdin("")
        .assert()
        .success();
    let installed = graphify(&dir)
        .args(["schedule", "--install"])
        .write_stdin("")
        .assert()
        .success();
    let printed = String::from_utf8_lossy(&printed.get_output().stdout).to_string();
    let installed = String::from_utf8_lossy(&installed.get_output().stdout).to_string();
    assert_ne!(printed, installed, "the two flags are interchangeable");
    assert!(
        printed.contains("crontab -e"),
        "--print did not print the crontab line:\n{printed}"
    );
    assert!(
        !printed.contains("[y/N]"),
        "--print asked to write something:\n{printed}"
    );
    assert!(
        installed.contains("[y/N]"),
        "--install did not ask before writing:\n{installed}"
    );
}
