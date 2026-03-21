#!/usr/bin/env bash
# One-command install for amnezia-xray-admin Telegram bot on VPS.
# Usage: curl -sSL <url>/install.sh | bash -s -- <TELEGRAM_TOKEN>
#   or:  ./install.sh <TELEGRAM_TOKEN> [XRAY_CONTAINER]

set -euo pipefail

TELEGRAM_TOKEN="${1:-}"
XRAY_CONTAINER="${2:-amnezia-xray}"
IMAGE_NAME="axadmin"
CONTAINER_NAME="axadmin-bot"
REPO_URL="https://github.com/AmneziaVPN/amnezia-xray-admin.git"

if [ -z "$TELEGRAM_TOKEN" ]; then
    echo "Usage: $0 <TELEGRAM_TOKEN> [XRAY_CONTAINER]"
    echo ""
    echo "  TELEGRAM_TOKEN   - Bot token from @BotFather"
    echo "  XRAY_CONTAINER   - Xray container name (default: amnezia-xray)"
    exit 1
fi

echo "==> Checking Docker..."
if ! command -v docker &>/dev/null; then
    echo "ERROR: Docker is not installed. Install Docker first."
    exit 1
fi

echo "==> Checking Xray container '$XRAY_CONTAINER'..."
if ! docker inspect "$XRAY_CONTAINER" &>/dev/null; then
    echo "WARNING: Container '$XRAY_CONTAINER' not found. The bot will fail to connect."
    echo "         Make sure Amnezia VPN is set up and the container is running."
fi

echo "==> Stopping existing bot (if any)..."
docker rm -f "$CONTAINER_NAME" 2>/dev/null || true

echo "==> Building image..."
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
git clone --depth 1 "$REPO_URL" "$TMPDIR/repo"
docker build -t "$IMAGE_NAME" "$TMPDIR/repo"

echo "==> Starting bot..."
docker run -d \
    --name "$CONTAINER_NAME" \
    --restart unless-stopped \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -e "TELEGRAM_TOKEN=$TELEGRAM_TOKEN" \
    "$IMAGE_NAME" \
    --telegram-bot --local --container "$XRAY_CONTAINER"

echo ""
echo "==> Bot started successfully!"
echo "    Container: $CONTAINER_NAME"
echo "    Logs: docker logs -f $CONTAINER_NAME"
echo ""
echo "    Send /start to your bot in Telegram to become the admin."
