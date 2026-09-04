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

## In a container

One image holds the whole product: the dashboard compiled into the engine binary, the
Python brain beside it, and a cron for the daily sync. It needs a password, because
unlike a local run it is listening on a published port.

    GRAPHIFY_PASSWORD='something long' docker compose up --build

Then open `http://localhost:3737` and type the password. Put the line in a `.env` file
beside `docker-compose.yml` to stop typing it; git already ignores that file. Compose
refuses to start without a password rather than serving your calls to whoever asks.

The database and the key file live in a Docker volume mounted at `/data`, so they survive
`docker compose down`. `docker compose down -v` deletes them both.

**Keys are never in the image.** `docker history` prints every environment variable of
every layer, so anything baked in is readable by anyone who can pull it — and stays
readable after it is rotated. Add your Vapi and model keys through the settings page,
which encrypts them into the database, or pass them in the environment. Compose passes
`VAPI_API_KEY`, `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` through only when your shell
has them set.

`GRAPHIFY_SECRET` is what those stored keys are encrypted under. Leave it unset and one
is generated into `/data/.secret` on first run: fine, until the volume is deleted, after
which a backup of the database alone decrypts to nothing. Set it to keep the key
somewhere the volume is not.

The sync runs at 06:00 in the container's timezone, which is UTC unless you pass `TZ`.
`GRAPHIFY_CRON` takes a crontab expression, or `off` for an image that only serves.

The port is published on `127.0.0.1` only. There is no TLS in the image, so the password
crosses the connection in clear: reaching this from another machine means a reverse proxy
with a certificate in front of it, not a wider `ports:` line.
