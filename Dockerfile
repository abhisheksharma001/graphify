# syntax=docker/dockerfile:1

# The whole product in one image: the dashboard compiled into the engine binary, the
# brain beside it, and a cron that is not the machine's cron.
#
# Three build stages because three toolchains are needed to build it and none is needed
# to run it. What ships is a Python image with two binaries in it — no node, no cargo, no
# Rust source, no pnpm store.
#
# Nothing below is a key. `docker history` prints every ENV and every ARG of every layer,
# so a key written into this file would be readable by anyone who can pull the image, and
# would stay readable after it was rotated. Keys arrive at run time only: through the
# environment, or through the settings page, which puts them in the database encrypted.

# ---- the dashboard -----------------------------------------------------------------
#
# First, because the engine eats it. `engine/build.rs` reads `../ui/dist` at compile time
# and embeds what it finds, so a cargo build that runs before this one produces a working
# binary that serves a page saying the UI was never built.
FROM node:22-slim AS ui
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
WORKDIR /ui
RUN corepack enable
# The manifest and the lockfile on their own, so the install layer survives every change
# to the source and is rebuilt only when a dependency actually moves. `pnpm-workspace.yaml`
# comes with them and is not optional: it carries the `allowBuilds` decision about
# `core-js`, and pnpm refuses to install at all when an ignored build script has no answer
# recorded and nobody to ask.
COPY ui/package.json ui/pnpm-lock.yaml ui/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
RUN pnpm build

# ---- the engine --------------------------------------------------------------------
#
# Debian and not Alpine: `rusqlite`'s `bundled` feature compiles SQLite from C, and the
# runtime image is `python:3.12-slim`, which is glibc. A musl binary built on Alpine
# would build fine here and then not run there.
FROM rust:1-slim-bookworm AS engine
WORKDIR /src
COPY engine/ engine/
COPY --from=ui /ui/dist ui/dist
WORKDIR /src/engine
# The `cp` is inside the same RUN because `target/` is a cache mount: it exists while this
# command runs and is gone from the layer afterwards. Copying the binary out is the only
# way anything survives.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/engine/target \
    cargo build --release --locked \
 && cp target/release/graphify /usr/local/bin/graphify

# ---- the container's cron ----------------------------------------------------------
#
# supercronic rather than Debian's `cron`, which wants to be PID 1, throws away the
# environment it was started with, and mails its output to a spool nobody reads. This one
# is an ordinary process that hands each job the environment it inherited — which is how
# GRAPHIFY_SECRET reaches the six o'clock sync — and logs to stderr, where `docker logs`
# is already looking.
FROM debian:bookworm-slim AS supercronic
ARG TARGETARCH
ARG SUPERCRONIC_VERSION=v0.2.49
# One checksum per architecture, because the checksum is of a binary and not of a
# release: a build on an arm64 Mac and a build on an amd64 server pull different files,
# and "whatever came down the wire" is not a check.
ARG SUPERCRONIC_SHA256_amd64=a53ae236602c7338aba3fbaff40bda6300eae3b9fedb8261eb06cfe3724430c1
ARG SUPERCRONIC_SHA256_arm64=02aa0cb229ba09050cba6638059dadb9eedc2276632ea43d6a57a2f8c1629dd5
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    eval "sha=\${SUPERCRONIC_SHA256_${TARGETARCH}:-}"; \
    [ -n "$sha" ] || { echo "no supercronic checksum recorded for ${TARGETARCH}" >&2; exit 1; }; \
    curl -fsSLo /usr/local/bin/supercronic \
      "https://github.com/aptible/supercronic/releases/download/${SUPERCRONIC_VERSION}/supercronic-linux-${TARGETARCH}"; \
    echo "$sha  /usr/local/bin/supercronic" | sha256sum -c -; \
    chmod +x /usr/local/bin/supercronic

# ---- what actually ships -----------------------------------------------------------
FROM python:3.12-slim-bookworm

# The same lockfile CI resolves against. `--frozen` so the image is built from `uv.lock`
# and not from whatever happened to resolve on the day it was built.
RUN pip install --no-cache-dir uv

WORKDIR /app/brain
COPY brain/pyproject.toml brain/uv.lock brain/README.md ./
# Dependencies before the source, so editing one Python file does not re-resolve the
# world. `--no-install-project` because the project itself is not here yet.
RUN uv sync --frozen --no-dev --no-install-project
COPY brain/ ./
# `baml_client/` is generated and gitignored, so it has to be written here or every model
# call in the image fails on an import. It has to be written after the source arrives,
# because it is generated from `baml_src/`.
RUN uv sync --frozen --no-dev \
 && uv run baml-cli generate

# The venv's `bin` on PATH is how the engine finds `graphify-brain`: it spawns the name,
# unqualified, and `GRAPHIFY_BRAIN` exists to override that and is not needed here.
ENV PATH="/app/brain/.venv/bin:$PATH"

COPY --from=engine /usr/local/bin/graphify /usr/local/bin/graphify
COPY --from=supercronic /usr/local/bin/supercronic /usr/local/bin/supercronic
COPY --chmod=755 docker/entrypoint.sh /usr/local/bin/graphify-entrypoint

# Not root. This image listens on every interface it has, and the one thing it is allowed
# to write is the volume. Creating `/data` here and giving it away is also what makes the
# volume writable: Docker copies this directory's ownership onto a named volume the first
# time it fills one.
RUN useradd --create-home --uid 10001 graphify \
 && mkdir -p /data \
 && chown graphify:graphify /data

# `0.0.0.0` because the default, `127.0.0.1`, is the container's own loopback, which
# nothing outside the container can reach. Which interface of the *host* this is
# published on is the host's decision, and `docker-compose.yml` publishes it on loopback.
ENV GRAPHIFY_DB=/data/graphify.db \
    GRAPHIFY_BIND=0.0.0.0:3737

VOLUME /data
EXPOSE 3737
USER graphify
WORKDIR /data

ENTRYPOINT ["/usr/local/bin/graphify-entrypoint"]
CMD ["serve", "--no-open"]
