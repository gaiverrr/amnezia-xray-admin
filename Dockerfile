# Minimal runtime — static musl binary, only needs Docker CLI
FROM alpine:3.21

RUN apk add --no-cache docker-cli curl

COPY amnezia-xray-admin /usr/local/bin/amnezia-xray-admin

ENTRYPOINT ["amnezia-xray-admin"]
CMD ["--telegram-bot", "--local", "--container", "amnezia-xray"]
