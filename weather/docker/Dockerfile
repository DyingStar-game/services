# Build context = the `services/` directory (the weather service needs the celestial data file of
# resourcesDynamic, which is the single source of the system's physical parameters):
#   docker build -f weather/docker/Dockerfile -t weather:dev .
# `weather/docker/Dockerfile.dockerignore` keeps the other services out of the context.

# ─── Stage 1: build ────────────────────────────────────────────────────────
FROM rust:1.95-slim AS builder

WORKDIR /app

# Cache the dependencies: manifests first, stub sources, build, then the real sources.
COPY weather/Cargo.toml weather/Cargo.lock* ./
COPY weather/crates/weather-core/Cargo.toml crates/weather-core/Cargo.toml
COPY weather/crates/ds-weather-proto/Cargo.toml crates/ds-weather-proto/Cargo.toml
COPY weather/crates/weather-service/Cargo.toml crates/weather-service/Cargo.toml
RUN mkdir -p crates/weather-core/src crates/ds-weather-proto/src crates/weather-service/src \
    && echo 'pub fn stub() {}' > crates/weather-core/src/lib.rs \
    && echo 'pub fn stub() {}' > crates/ds-weather-proto/src/lib.rs \
    && echo 'fn main() {}' > crates/weather-service/src/main.rs \
    && cargo build --release -p weather-service \
    && rm -rf crates/*/src

COPY weather/crates ./crates
RUN touch crates/*/src/*.rs && cargo build --release -p weather-service

# ─── Stage 2: minimal runtime ──────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
RUN groupadd -g 1001 appgroup && useradd -u 1001 -g appgroup -s /bin/sh -m appuser

WORKDIR /app
COPY --from=builder /app/target/release/weather ./weather
# The system's physical parameters (one source, shared with resourcesDynamic) and the weather's own knobs.
COPY resourcesDynamic/data/system ./data/system
COPY weather/data/climate ./data/climate

RUN chown -R appuser:appgroup /app
USER appuser

ENV PORT=9400 \
    SYSTEM_JSON=data/system/tarsis.json \
    CLIMATE_JSON=data/climate/tarsis.json \
    CELESTIAL_HTTP=http://service-resourcesdynamic:3001 \
    RUST_LOG=weather=info

EXPOSE 9400
ENTRYPOINT ["./weather"]
