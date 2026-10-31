# ─── Stage 1: build ────────────────────────────────────────────────────────
# Alpine = musl target: the binary comes out fully static, so the runtime image
# needs no libc at all. TLS is rustls everywhere (no openssl-sys in Cargo.lock);
# musl-dev is for the C parts of ring and mimalloc.
FROM rust:1.95-alpine AS builder

RUN apk add --no-cache musl-dev

WORKDIR /app

# Cache dependencies separately from source code.
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release && rm -rf src

# Copy real source and rebuild. The migrations are embedded at compile time
# (`sqlx::migrate!`).
COPY migrations ./migrations
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ─── Stage 2: minimal runtime ──────────────────────────────────────────────
# Distroless static: CA certificates, tzdata and a nonroot user (65532), no
# libc, no shell, no package manager — nearly no OS-package CVE surface.
FROM gcr.io/distroless/static-debian12:nonroot AS runtime

WORKDIR /app
COPY --from=builder /app/target/release/npc ./npc
COPY config ./config

EXPOSE 9400

ENTRYPOINT ["./npc"]
