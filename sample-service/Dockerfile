# syntax=docker/dockerfile:1
# The sample node. Build from the repository root:
#   docker build -f sample-service/Dockerfile -t cicada-rs-sample .
# Multi-arch: both stages use multi-platform base images, so this builds natively on arm64 (OrbStack, Apple silicon)
# and amd64.

# ---- build ----
FROM rust:1.94-slim-bookworm AS build
WORKDIR /src

COPY . .
# The registry and the target folder are build caches, so a source change rebuilds only what changed.
# The binary is copied out of the cached target folder, which is not part of the image.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p cicada-sample-service \
 && cp target/release/cicada-sample-service /usr/local/bin/cicada-sample-service

# ---- runtime: Debian slim ----
FROM debian:bookworm-slim
# CA certificates: the system's list, which the node itself does not read. Its OTLP exporter speaks plain HTTP only
# (its reqwest has no TLS feature), and the PostgreSQL driver has its own list of public CAs and takes a private one
# from `sslrootcert`. curl only for the container health check.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --user-group --no-create-home cicada \
 && mkdir -p /data && chown cicada:cicada /data

COPY --from=build /usr/local/bin/cicada-sample-service /usr/local/bin/cicada-sample-service
USER cicada
WORKDIR /data

# CICADA_SHUTDOWN_GRACE_PERIOD: on `docker stop` the node stops claiming and lets running turns finish for this long
# while the API still answers, then cancels the rest (up to 5 s more) and releases their leases; then the HTTP server
# stops. The library default (30 s) does not fit in the 10 s `docker stop` waits by default before it kills the
# process (and its leases then expire instead of being released), so the image defaults to 3 s. For longer turns raise
# both: e.g. CICADA_SHUTDOWN_GRACE_PERIOD=30s with `docker stop -t 45` / `docker run --stop-timeout 45` / compose
# `stop_grace_period: 45s`. CICADA_LISTEN opens the API to every network of the container: `serve` listens on the
# loopback address unless told otherwise. It is set here and not in a compose file, because the end-to-end suite also
# starts nodes with `docker run`. CICADA_STORE is the SQLite file on the volume; set it to a postgres:// address for
# PostgreSQL.
ENV CICADA_LISTEN=0.0.0.0:8080 \
    CICADA_STORE=/data/actors.db \
    CICADA_SHUTDOWN_GRACE_PERIOD=3s \
    RUST_LOG=info,sqlx=warn
EXPOSE 8080
VOLUME /data

# GET /ready is public (App answers it outside the host's router), so the check needs no token. It asks the port of
# CICADA_LISTEN.
HEALTHCHECK --interval=5s --timeout=3s --start-period=30s --retries=5 \
  CMD curl -fsS "http://localhost:${CICADA_LISTEN##*:}/ready" > /dev/null || exit 1

# The binary is PID 1 and handles SIGTERM: the node stops in good order (no new claims, running turns finish within
# CICADA_SHUTDOWN_GRACE_PERIOD, leases released), then the HTTP server.
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/cicada-sample-service", "serve"]
