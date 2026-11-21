# ── Stage 1: build ────────────────────────────────────────────────────────────
FROM rust:1.95-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies separately by building a stub binary first.
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release && rm -rf src

# Build the real binary (migrations and openapi.yaml are embedded).
COPY src ./src
COPY migrations ./migrations
COPY openapi.yaml ./openapi.yaml
RUN touch src/main.rs && cargo build --release

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -u 1001 -m appuser

WORKDIR /app
COPY --from=builder /app/target/release/universesimulator ./universesimulator
COPY seeds ./seeds

USER appuser

EXPOSE 9600

CMD ["./universesimulator"]
