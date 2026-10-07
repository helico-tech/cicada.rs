# syntax=docker/dockerfile:1
# The sample node. Build from the repository root:
#   docker build -f sample-service/Dockerfile -t cicada-rs-sample .
# Multi-arch: both stages use multi-platform base images, so this builds natively on arm64 (OrbStack, Apple silicon)
# and amd64.

# ---- build: Rust, and Node 22 for the console (cicada-ui builds it with npm and Vite) ----
FROM rust:1.94-slim-bookworm AS build
COPY --from=node:22-slim /usr/local/bin/node /usr/local/bin/node
COPY --from=node:22-slim /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm && node --version && npm --version
WORKDIR /src

COPY . .
# The registry, the target folder and npm's cache are build caches, so a source change rebuilds only what changed.
# The binary is copied out of the cached target folder, which is not part of the image.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/root/.npm \
    cargo build --release --locked -p cicada-sample-service \
 && cp target/release/cicada-sample-service /usr/local/bin/cicada-sample-service

# ---- runtime: Debian slim ----
FROM debian:bookworm-slim
# CA certificates for TLS to PostgreSQL and OTLP endpoints; curl only for the container health check.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --user-group --no-create-home cicada \
 && mkdir -p /data && chown cicada:cicada /data

COPY --from=build /usr/local/bin/cicada-sample-service /usr/local/bin/cicada-sample-service
USER cicada
WORKDIR /data

# SHUTDOWN_GRACE_PERIOD: on `docker stop` the node lets running turns finish for this long, then cancels the rest
# (up to 5 s more) and releases their leases; the HTTP server stops accepting requests first. The library default
# (30 s) does not fit in the 10 s `docker stop` waits by default before it kills the process (and its leases then expire
# instead of being released), so the image defaults to 3 s. For longer turns raise both: e.g.
# SHUTDOWN_GRACE_PERIOD=30s with `docker stop -t 45` / `docker run --stop-timeout 45` / compose `stop_grace_period: 45s`.
ENV PORT=8080 \
    SQLITE_PATH=/data/actors.db \
    SHUTDOWN_GRACE_PERIOD=3s \
    RUST_LOG=info,sqlx=warn
EXPOSE 8080
VOLUME /data

# GET /ready is public (actor_health()), so the check needs no token.
HEALTHCHECK --interval=5s --timeout=3s --start-period=30s --retries=5 \
  CMD curl -fsS "http://localhost:${PORT}/ready" > /dev/null || exit 1

# The binary is PID 1 and handles SIGTERM: the node stops gracefully (no new claims, running turns finish within
# SHUTDOWN_GRACE_PERIOD, leases released).
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/cicada-sample-service"]
