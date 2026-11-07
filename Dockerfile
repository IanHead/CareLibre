# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Build stage
# ---------------------------------------------------------------------------
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# Cache dependencies separately from application code.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations

RUN cargo build --release --locked --bin carelibre-server

# ---------------------------------------------------------------------------
# Runtime stage
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 carelibre \
    && useradd --system --uid 10001 --gid carelibre --no-create-home carelibre

COPY --from=builder /build/target/release/carelibre-server /usr/local/bin/carelibre-server

USER 10001:10001
EXPOSE 8080

ENV BIND_ADDR=0.0.0.0:8080 \
    APP_ENV=production

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["carelibre-server"]
