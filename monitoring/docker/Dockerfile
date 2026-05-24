# ─── Stage 1: build ────────────────────────────────────────────────────────
FROM rust:1.87-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies separately from source code.
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release && rm -rf src

# Copy real source and rebuild.
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ─── Stage 2: minimal runtime ──────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

RUN useradd -r -u 1001 -g root monitoring
WORKDIR /app

COPY --from=builder /app/target/release/monitoring /app/monitoring

USER 1001

EXPOSE 9300

ENTRYPOINT ["/app/monitoring"]
