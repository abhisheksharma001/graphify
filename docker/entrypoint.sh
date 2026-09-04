#!/bin/sh
# Starts the container's morning before it starts the container's server.
#
# S-31 prints a crontab line with every path spelled out, because a laptop's cron has no
# working directory and a PATH of `/usr/bin:/bin`. A container has neither problem: PATH,
# GRAPHIFY_DB and GRAPHIFY_BIND are set in the image, and supercronic hands each job the
# environment it was started with — including GRAPHIFY_SECRET, which the printed line
# cannot carry because a key is never printed. So the line here is the short one. The
# absolutes it does not need are the absolutes the image already supplies.
set -eu

# A crontab expression, or `off` for an image that only serves.
CRON="${GRAPHIFY_CRON:-0 6 * * *}"

# Only alongside the server. `docker compose run graphify sync --org acme` is one command
# that is supposed to end, not a second scheduler that outlives it.
if [ "${1:-}" = "serve" ] && [ "$CRON" != "off" ]; then
    printf '%s graphify sync --org all\n' "$CRON" > /tmp/graphify.crontab
    supercronic /tmp/graphify.crontab &
fi

exec graphify "$@"
