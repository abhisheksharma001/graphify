//! What a machine needs in order to run graphify every morning, written out as text.
//!
//! Nothing in the first half of this file schedules anything: a `Plan` turns into a
//! crontab line and into a launchd plist, and that is all. They are pure because a
//! scheduler cannot be checked by running it — a crontab line does nothing until
//! tomorrow, and the only thing there is to look at today is what was written.
//!
//! Three things go wrong with a scheduled job, and all three are the same thing: the
//! environment a terminal gives you for free is not there.
//!
//! **There is no working directory.** cron runs from `/`, launchd from `/`. The default
//! database path is `data/graphify.db`, which is relative, so an unqualified line does not
//! fail — it quietly makes an empty database somewhere else and syncs into that. Every
//! path here is absolute for that reason, and the acceptance for this step is exactly it.
//!
//! **There is no PATH worth having.** cron's is `/usr/bin:/bin`. Neither `graphify` nor
//! the `graphify-brain` it spawns is there, so both are resolved now, while a shell that
//! knows where they are is still running, and written out in full.
//!
//! **There is no environment.** In particular there is no `GRAPHIFY_SECRET`. A key is
//! never printed, so if one is set in the shell that runs this, the scheduled job will
//! fall back to the key file — a different key, under which every stored Vapi key fails
//! to decrypt. That is a warning rather than a value: see `notes`.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The launchd job's name, and the file it is saved as. Reversed-domain, because launchd
/// takes the label as the identity of the job: loading a second one under the same label
/// replaces the first rather than doubling the morning.
pub const LABEL: &str = "ai.graphify.daily";

/// Tail of the crontab line, and the way `--install` finds its own previous work. cron
/// hands the command to `/bin/sh`, so a `#` comment at the end of it is the shell's and
/// costs nothing — which makes one line both the job and its own marker.
pub const MARKER: &str = "# graphify schedule";

/// One morning run, with every path already resolved.
pub struct Plan {
    /// This binary, absolute. Not "graphify" — see the module note about PATH.
    pub binary: PathBuf,
    /// The database the run should open, absolute. The key file is its sibling, so this
    /// one variable settles both.
    pub db: PathBuf,
    /// The brain, if a shell can find one now. `None` is not fatal — sync still runs and
    /// rules still re-count — but the day's labelling is the part that will not.
    pub brain: Option<PathBuf>,
    /// Where the run's own output goes. Next to the database, because that is the
    /// directory the operator already knows about.
    pub log: PathBuf,
    /// Which org, or `all`.
    pub org: String,
    pub hour: u32,
    pub minute: u32,
}

impl Plan {
    /// A plan for this machine as it stands: this binary, this database, this PATH.
    pub fn here(org: String, at: &str) -> Result<Self> {
        let (hour, minute) = parse_at(at)?;
        let binary = absolute(&std::env::current_exe().context("finding this binary")?)?;
        let db = absolute(&crate::db::default_path())?;
        Ok(Self {
            log: db.with_file_name("schedule.log"),
            brain: which(&crate::jobs::binary_from_env()),
            binary,
            db,
            org,
            hour,
            minute,
        })
    }

    /// One crontab line: five time fields, then a command `/bin/sh` can read.
    pub fn crontab(&self) -> String {
        let mut env = format!("GRAPHIFY_DB={}", quote(&self.db));
        if let Some(brain) = &self.brain {
            env.push_str(&format!(" GRAPHIFY_BRAIN={}", quote(brain)));
        }
        format!(
            "{} {} * * * {env} {} sync --org {} >> {} 2>&1 {MARKER}",
            self.minute,
            self.hour,
            quote(&self.binary),
            quote_str(&self.org),
            quote(&self.log),
        )
    }

    /// The same job as a launchd agent. `StartCalendarInterval` rather than an interval:
    /// a Mac that was asleep at six runs it on waking instead of skipping the day.
    pub fn plist(&self) -> String {
        let mut env = format!(
            "    <key>GRAPHIFY_DB</key>\n    <string>{}</string>\n",
            xml(&self.db.display().to_string())
        );
        if let Some(brain) = &self.brain {
            env.push_str(&format!(
                "    <key>GRAPHIFY_BRAIN</key>\n    <string>{}</string>\n",
                xml(&brain.display().to_string())
            ));
        }
        let log = xml(&self.log.display().to_string());
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{binary}</string>
    <string>sync</string>
    <string>--org</string>
    <string>{org}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
{env}  </dict>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Hour</key>
    <integer>{hour}</integer>
    <key>Minute</key>
    <integer>{minute}</integer>
  </dict>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>RunAtLoad</key>
  <false/>
</dict>
</plist>
"#,
            binary = xml(&self.binary.display().to_string()),
            org = xml(&self.org),
            hour = self.hour,
            minute = self.minute,
        )
    }

    /// What is true about this machine that the two blocks above cannot say themselves.
    /// Never a value: the first of these exists precisely because a key is not printed.
    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if std::env::var("GRAPHIFY_SECRET").is_ok_and(|v| !v.trim().is_empty()) {
            out.push(format!(
                "GRAPHIFY_SECRET is set in this shell and is not written above, because a key is \
                 never printed. The scheduled run will not have it and will fall back to {}, \
                 which is a different key — every stored Vapi key would fail to decrypt under it. \
                 Put GRAPHIFY_SECRET into the scheduled job's environment yourself, or unset it \
                 here and let the file be the one key.",
                self.db.with_file_name(".secret").display()
            ));
        }
        if self.brain.is_none() {
            out.push(
                "graphify-brain is not on PATH, so the run will sync and re-count every rule but \
                 the day's labelling will fail. Install it, or set GRAPHIFY_BRAIN to its path and \
                 print this again."
                    .to_string(),
            );
        }
        out
    }
}

/// Print both forms and write nothing. The default, and the whole of `--print`.
pub fn print(plan: &Plan) {
    println!(
        "At {:02}:{:02} every day: pull new calls, refresh assistants, re-run every rule, then \
         label what is new inside GRAPHIFY_DAILY_CAP_USD.\n",
        plan.hour, plan.minute
    );
    println!("cron — `crontab -e`, then this one line:\n");
    println!("{}\n", plan.crontab());
    println!(
        "launchd — save as ~/Library/LaunchAgents/{LABEL}.plist, then\n\
         `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/{LABEL}.plist`:\n"
    );
    println!("{}", plan.plist());
    for note in plan.notes() {
        println!("note: {note}\n");
    }
}

/// Write the one this machine actually uses — after asking, always.
pub fn install(plan: &Plan) -> Result<()> {
    for note in plan.notes() {
        println!("note: {note}\n");
    }
    if cfg!(target_os = "macos") {
        launchd(plan)
    } else if cfg!(target_os = "linux") {
        cron(plan)
    } else {
        bail!("--install knows macOS and Linux; `graphify schedule --print` gives you the line")
    }
}

fn launchd(plan: &Plan) -> Result<()> {
    let dir = home()?.join("Library/LaunchAgents");
    let path = dir.join(format!("{LABEL}.plist"));
    println!("{}", plan.plist());
    println!("That goes to {}.", path.display());
    if path.exists() {
        println!("There is one there already and it will be replaced.");
    }
    if !confirm("Write it and load it?")? {
        println!("Nothing written.");
        return Ok(());
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(&path, plan.plist()).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());

    let uid = run("id", &["-u".into()])?.trim().to_string();
    // Unloading first is how launchd is told to replace rather than refuse: `bootstrap`
    // on a label that is already loaded is an error, and on the first install there is
    // nothing to unload, so this one is allowed to fail.
    let _ = run("launchctl", &["bootout".into(), format!("gui/{uid}/{LABEL}")]);
    match run(
        "launchctl",
        &[
            "bootstrap".into(),
            format!("gui/{uid}"),
            path.display().to_string(),
        ],
    ) {
        Ok(_) => println!(
            "loaded {LABEL}; the first run is tomorrow at {:02}:{:02}",
            plan.hour, plan.minute
        ),
        Err(e) => println!(
            "the plist is written but launchd would not take it: {e:#}\n\
             load it by hand with `launchctl bootstrap gui/{uid} {}`",
            path.display()
        ),
    }
    Ok(())
}

fn cron(plan: &Plan) -> Result<()> {
    let line = plan.crontab();
    // No crontab at all is `crontab -l` exiting 1, which is not an error here — it is the
    // ordinary state of a machine that has never had one.
    let existing = run("crontab", &["-l".into()]).unwrap_or_default();
    let kept: Vec<&str> = existing
        .lines()
        .filter(|l| !l.contains(MARKER))
        .collect::<Vec<_>>();
    let replaced = existing.lines().count() - kept.len();

    println!("{line}\n");
    println!("That goes into your crontab.");
    if replaced > 0 {
        println!("{replaced} line(s) graphify put there before will be replaced.");
    }
    if !confirm("Write it?")? {
        println!("Nothing written.");
        return Ok(());
    }
    let mut next = kept.join("\n");
    if !next.is_empty() {
        next.push('\n');
    }
    next.push_str(&line);
    next.push('\n');
    write_stdin("crontab", &["-".into()], &next)?;
    println!(
        "installed; the first run is tomorrow at {:02}:{:02}",
        plan.hour, plan.minute
    );
    Ok(())
}

/// Nothing is written unless the answer is yes. A closed stdin — a pipe, a CI job, an
/// `--install` in a script — reads as end of input, which is not a yes.
fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer)? == 0 {
        return Ok(false);
    }
    let answer = answer.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

fn run(program: &str, args: &[String]) -> Result<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn write_stdin(program: &str, args: &[String], input: &str) -> Result<()> {
    use std::process::Stdio;
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {program}"))?;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(input.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        bail!("{program} exited {status}");
    }
    Ok(())
}

fn home() -> Result<PathBuf> {
    match std::env::var_os("HOME") {
        Some(h) => Ok(PathBuf::from(h)),
        None => bail!("HOME is not set, so there is no ~/Library/LaunchAgents to write to"),
    }
}

/// `HH:MM`, on this machine's clock. Rejected here rather than by cron, which would
/// simply never run.
pub fn parse_at(at: &str) -> Result<(u32, u32)> {
    let (h, m) = at
        .split_once(':')
        .with_context(|| format!("--at wants HH:MM, got {at}"))?;
    let hour: u32 = h
        .parse()
        .with_context(|| format!("--at wants HH:MM, got {at}"))?;
    let minute: u32 = m
        .parse()
        .with_context(|| format!("--at wants HH:MM, got {at}"))?;
    if hour > 23 || minute > 59 {
        bail!("--at wants a time of day, got {at}");
    }
    Ok((hour, minute))
}

/// A path as given if it is already absolute, else joined to where we are standing now.
/// Not `canonicalize`: the log file does not exist yet, and neither does the database on
/// a machine being set up.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("finding the working directory")?
        .join(path))
}

/// Where a shell would find this program right now. A name with a slash in it is already
/// a path and is only made absolute.
fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return absolute(Path::new(name)).ok();
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// A path the way `/bin/sh` must read it. Single quotes take everything literally, and
/// the one character they cannot hold is closed, escaped, and reopened.
fn quote(path: &Path) -> String {
    quote_str(&path.display().to_string())
}

fn quote_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
