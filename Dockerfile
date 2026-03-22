# Minimal runtime image — binary is pre-built and pulled from ghcr.io
# Only needs Docker CLI to exec into the xray container
FROM debian:bookworm-slim

# Install Docker CLI only (not the daemon) — download official static binary
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    curl -fsSL https://download.docker.com/linux/static/stable/x86_64/docker-27.5.1.tgz | \
    tar xz --strip-components=1 -C /usr/local/bin docker/docker && \
    apt-get purge -y curl && \
    apt-get autoremove -y && \
    rm -rf /var/lib/apt/lists/*

COPY amnezia-xray-admin /usr/local/bin/amnezia-xray-admin

ENTRYPOINT ["amnezia-xray-admin"]
CMD ["--telegram-bot", "--local", "--container", "amnezia-xray"]
