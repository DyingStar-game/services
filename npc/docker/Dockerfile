# ─── Stage 1: build ────────────────────────────────────────────────────────
FROM rust:1.95-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

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

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -g 1001 appgroup && useradd -u 1001 -g appgroup -s /bin/sh -m appuser

WORKDIR /app
COPY --from=builder /app/target/release/npc ./npc
COPY config ./config

RUN chown -R appuser:appgroup /app
USER appuser

EXPOSE 9400

ENTRYPOINT ["./npc"]
