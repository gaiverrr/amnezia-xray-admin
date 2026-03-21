# Stage 1: Build the Rust binary
FROM rust:latest AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY Dockerfile ./
COPY src/ src/

RUN cargo build --release --bin amnezia-xray-admin

# Stage 2: Minimal runtime with Docker CLI
FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        docker.io && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/amnezia-xray-admin /usr/local/bin/amnezia-xray-admin

ENTRYPOINT ["amnezia-xray-admin"]
CMD ["--telegram-bot", "--local", "--container", "amnezia-xray"]
