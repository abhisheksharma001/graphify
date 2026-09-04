//! A scheduler cannot be checked by running it — the crontab line does nothing until
//! tomorrow. What there is to check today is the text, and every one of these is about a
//! way the text could be right in a terminal and wrong at six in the morning.

use assert_cmd::Command;
use tempfile::TempDir;

/// A database path of its own, so the printed line is one we can predict, and none of the
/// variables the printing reads left over from the shell that ran the tests.
fn graphify(db: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("graphify").unwrap();
    cmd.env("GRAPHIFY_DB", db)
        .env_remove("GRAPHIFY_SECRET")
        .env_remove("GRAPHIFY_BRAIN");
    cmd
}

fn printed(dir: &TempDir, args: &[&str]) -> String {
    let out = graphify(&dir.path().join("data").join("graphify.db"))
        .args(args)
        .assert()
        .success();
    String::from_utf8_lossy(&out.get_output().stdout).to_string()
}

fn cron_line(text: &str) -> String {
    text.lines()
        .find(|l| l.contains("sync --org"))
        .unwrap_or_else(|| panic!("no crontab line in:\n{text}"))
        .to_string()
}

/// The acceptance for this step. cron's PATH is `/usr/bin:/bin` and `graphify` is not in
/// either of them, so a line that names the program is a line that never runs.
#[test]
fn the_crontab_line_names_the_binary_in_full() {
    let dir = tempfile::tempdir().unwrap();
    let line = cron_line(&printed(&dir, &["schedule", "--print"]));
    let binary = assert_cmd::cargo::cargo_bin("graphify");
    assert!(binary.is_absolute(), "{binary:?}");
    assert!(line.contains(&binary.display().to_string()), "line was: {line}");
}

/// The other absent thing is a working directory. `data/graphify.db` is relative, and a
/// job that runs from `/` does not fail on it — it makes an empty database somewhere else
/// and syncs into that.
#[test]
fn every_path_in_both_forms_is_absolute() {
    let dir = tempfile::tempdir().unwrap();
    let text = printed(&dir, &["schedule", "--print"]);
    let db = dir.path().join("data").join("graphify.db");
    let log = dir.path().join("data").join("schedule.log");
    for wanted in [db.display().to_string(), log.display().to_string()] {
        assert!(text.contains(&wanted), "{wanted} missing from:\n{text}");
    }
    assert!(!cron_line(&text).contains(" data/graphify.db"), "{text}");
}

#[test]
fn the_time_of_day_moves_both_forms_together() {
    let dir = tempfile::tempdir().unwrap();
    let text = printed(&dir, &["schedule", "--print", "--at", "07:30"]);
    assert!(cron_line(&text).starts_with("30 7 * * * "), "{text}");
    assert!(text.contains("<key>Hour</key>\n    <integer>7</integer>"), "{text}");
    assert!(text.contains("<key>Minute</key>\n    <integer>30</integer>"), "{text}");
}

/// Rejected here, where somebody is watching. A time cron cannot read is a job that
/// simply never runs, and nothing says so.
#[test]
fn a_time_that_is_not_a_time_is_refused_by_name() {
    for bad in ["25:00", "06:61", "six", "6"] {
        let dir = tempfile::tempdir().unwrap();
        let out = graphify(&dir.path().join("graphify.db"))
            .args(["schedule", "--print", "--at", bad])
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
        assert!(stderr.contains(bad), "stderr for {bad} was: {stderr}");
    }
}

/// A key is never printed — not even into a file the operator is about to paste into
/// their own crontab. What is printed instead is the consequence: the scheduled run will
/// fall back to the key file, which is a different key, and every stored Vapi key would
/// fail to decrypt under it.
#[test]
fn the_secret_is_never_printed_and_its_absence_is_explained() {
    let dir = tempfile::tempdir().unwrap();
    let secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let out = graphify(&dir.path().join("data").join("graphify.db"))
        .env("GRAPHIFY_SECRET", secret)
        .args(["schedule", "--print"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(!text.contains(secret), "the key was printed:\n{text}");
    assert!(!text.contains("GRAPHIFY_SECRET="), "the key was set in a line:\n{text}");
    assert!(text.contains("GRAPHIFY_SECRET is set in this shell"), "{text}");
}

/// One string goes to `/bin/sh` and another to an XML parser, and a directory with a
/// space and an ampersand in it breaks each of them differently.
#[test]
fn an_awkward_path_is_quoted_for_the_shell_and_escaped_for_the_plist() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("a b & c").join("graphify.db");
    let out = graphify(&db).args(["schedule", "--print"]).assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        cron_line(&text).contains(&format!("GRAPHIFY_DB='{}'", db.display())),
        "{text}"
    );
    assert!(
        text.contains(&db.display().to_string().replace('&', "&amp;")),
        "{text}"
    );
}

/// "Must not: install without confirm." A pipe with nothing in it reads as end of input,
/// which is the one answer that must never be taken for a yes.
#[test]
fn install_writes_nothing_unless_the_answer_is_yes() {
    for answer in ["n\n", "\n", ""] {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let out = graphify(&dir.path().join("data").join("graphify.db"))
            .env("HOME", &home)
            .args(["schedule", "--install"])
            .write_stdin(answer)
            .assert()
            .success();
        let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        assert!(text.contains("Nothing written."), "for {answer:?}: {text}");
        assert!(
            !home.join("Library/LaunchAgents").exists(),
            "a plist was written for {answer:?}"
        );
    }
}
