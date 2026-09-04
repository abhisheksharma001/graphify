# graphify

See how your Vapi voice agent is really doing. Paste an API key, pull your last
250 calls, get charts for how calls ended, which tool calls failed, where
transfers broke, and how latency moves over time.

Then describe what you care about in plain English — "calls where the caller
asked for a human", "calls where someone tried to book but couldn't" — and
graphify learns a rule from your calls once, then counts it every day for free.

Status: pre-alpha. Spec and step register in `docs/spec.md`.

## Every morning

One command prints the two ways a machine can run the daily sync — a crontab
line and a launchd job — with every path already filled in:

    graphify schedule --print

Both run `graphify sync --org all` at 06:00: pull new calls, refresh assistants,
re-run every rule, then label what is new inside `GRAPHIFY_DAILY_CAP_USD`. Only
the last of the four costs anything, and it stops at the cap. `--org` and
`--at HH:MM` change what gets printed.

`graphify schedule --install` writes the one this machine uses — launchd on
macOS, cron on Linux — after showing it and asking. It writes nothing unless the
answer is yes. Output goes to `schedule.log` beside the database.

A scheduled job starts with none of the environment a shell hands you: no
working directory, and a `PATH` of `/usr/bin:/bin`. That is why every path in
both forms is absolute and why `graphify-brain` is looked up while the line is
printed rather than when it runs.

The one thing that cannot be filled in is `GRAPHIFY_SECRET`, because a key is
never printed. If you keep yours in the environment, put it into the scheduled
job's environment yourself — otherwise the run falls back to the `.secret` file
beside the database, which is a different key, and every stored Vapi key fails
to decrypt under it. `schedule` says so when it finds one set.
