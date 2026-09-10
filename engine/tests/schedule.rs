//! A scheduler cannot be checked by running it — the crontab line does nothing until
//! tomorrow. What there is to check today is the text, and every one of these is about a
//! way the text could be right in a terminal and wrong at six in the morning.

use assert_cmd::Command;
use graphify::schedule::MARKER;
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

// S-52: the crontab line has two readers, not one. cron reads it before `/bin/sh` does,
// and an unescaped `%` ends the command there — the rest, `>> schedule.log` included, is
// fed to it as standard input.

/// Everything that has ever needed escaping for one of this file's readers, in one list so
/// the guard below covers them together. Each is used as an org name and as a directory
/// component of the database path, because the line carries both and a rule kept for one is
/// not kept for the other.
const HOSTILE: [&str; 10] = ["%", "a%b", "50% off", " ", "&", "'", "\"", "$x", "`x`", "\\"];

/// cron's own rule for the command field, mirroring the loop in cronie's `do_command.c`:
/// an unescaped `%` ends the command and everything after it is standard input, with the
/// later ones turned into newlines. A backslash before a `%` is removed and the `%` kept; a
/// backslash before anything else is passed through untouched — which is why `find … \;`
/// works in a crontab and why `quote_str`'s `'\''` is safe here. Written out rather than
/// asserted about, so the guard runs the parser instead of restating what it should do.
fn cron_reads(line: &str) -> (String, Option<String>) {
    let field = line.splitn(6, ' ').nth(5).expect("five time fields then a command");
    let mut command = String::new();
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'%') => {
                chars.next();
                command.push('%');
            }
            '%' => return (command, Some(chars.collect::<String>().replace('%', "\n"))),
            _ => command.push(c),
        }
    }
    (command, None)
}

/// `/bin/sh -n` reads a command and does not run it, which is the only way to ask the
/// second reader whether the first one left it something it can parse.
fn sh_accepts(command: &str) -> Result<(), String> {
    let out = std::process::Command::new("/bin/sh")
        .args(["-n", "-c", command])
        .output()
        .expect("running /bin/sh");
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The guard. Not "the line contains no bare `%`" — that would be this file agreeing with
/// itself. Both readers are run in the order cron runs them, and what has to survive is the
/// whole command, redirection and marker included.
#[test]
fn both_readers_of_the_crontab_line_get_something_they_can_parse() {
    for awkward in HOSTILE {
        for (what, org, db_dir) in [
            ("org name", awkward, "data"),
            ("database path", "acme", awkward),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let db = dir.path().join(db_dir).join("graphify.db");
            let out = graphify(&db)
                .args(["schedule", "--print", "--org", org])
                .assert()
                .success();
            let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
            let line = cron_line(&text);
            let (command, stdin) = cron_reads(&line);
            assert_eq!(
                stdin, None,
                "{what} {awkward:?}: cron cut the line and fed the rest to it as input\n{line}"
            );
            assert!(
                command.contains("schedule.log"),
                "{what} {awkward:?}: the redirection did not survive cron\n{line}"
            );
            if let Err(e) = sh_accepts(&command) {
                panic!("{what} {awkward:?}: the shell could not read what cron left:\n{command}\n{e}");
            }
        }
    }
}

/// The consequence, said once on its own: the sync does not run, and the file the operator
/// was told to read is the file that cannot receive the reason.
#[test]
fn a_percent_does_not_take_the_log_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("data").join("graphify.db");
    let out = graphify(&db)
        .args(["schedule", "--print", "--org", "50% club"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let (command, stdin) = cron_reads(&cron_line(&text));
    assert_eq!(stdin, None, "{text}");
    assert!(command.contains("sync --org '50% club'"), "{command}");
    assert!(command.trim_end().ends_with(MARKER), "{command}");
}

/// An org named `50% club` is a legal org. The line has to carry it, not refuse it.
#[test]
fn the_org_reaches_the_command_with_its_percent_intact() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("data").join("graphify.db");
    let out = graphify(&db)
        .args(["schedule", "--print", "--org", "a%b"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(cron_line(&text).contains(r"sync --org 'a\%b'"), "{text}");
}

/// The plist's one reader is an XML parser and launchd runs no shell, so the escape that
/// the crontab line needs would be a backslash inside the argument here.
#[test]
fn the_plist_is_not_escaped_for_a_reader_it_does_not_have() {
    for awkward in HOSTILE {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("data").join("graphify.db");
        let out = graphify(&db)
            .args(["schedule", "--print", "--org", awkward])
            .assert()
            .success();
        let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        let plist: String = text
            .lines()
            .skip_while(|l| !l.starts_with("<?xml"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!plist.is_empty(), "{text}");
        assert!(
            !plist.contains(r"\%"),
            "{awkward:?}: the plist was escaped for cron\n{plist}"
        );
        assert!(
            plist.contains(&format!("<string>{}</string>", awkward.replace('&', "&amp;"))),
            "{awkward:?}: the org did not reach the plist as itself\n{plist}"
        );
    }
}
