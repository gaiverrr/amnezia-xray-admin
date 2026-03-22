# Minimal runtime image — binary is cross-compiled locally and uploaded
# No Rust toolchain needed on VPS, no heavy compilation
FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        docker.io && \
    rm -rf /var/lib/apt/lists/*

COPY amnezia-xray-admin /usr/local/bin/amnezia-xray-admin

ENTRYPOINT ["amnezia-xray-admin"]
CMD ["--telegram-bot", "--local", "--container", "amnezia-xray"]
