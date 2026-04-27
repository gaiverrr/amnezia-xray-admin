# 3x-ui side-by-side panel on bridge

A second xray-management stack runs on `yc-vm` alongside production. It's
installed for two reasons: A/B testing of disconnect-prone xhttp/Reality
configurations, and a web UI for user management.

This doc tells operators what's there, how to access, how to verify health,
how to roll back. Sensitive credentials (admin password, Reality private
key, share URLs) are NOT in this repo — they live on Yuriy's Mac at
`~/.config/3x-ui-bridge/`.

## Layout

```
yc-vm (81.26.189.136)
├── xray.service [systemd]                    PRODUCTION  (port 443, 13 users)
│   + bot-socks inbound on 127.0.0.1:8119     (added 2026-04-27 for 3x-ui bot)
│
└── docker container `3x-ui` v2.9.2           TEST PANEL
    listen 127.0.0.1:2053                     web UI (SSH-tunnel only)
    listen 127.0.0.1:2096                     subscription server
    listen *:9443                              vless+xhttp+reality test inbound
    volumes: /opt/3x-ui/db, /opt/3x-ui/cert
    network_mode: host
```

YC SG `enpm15cfu8f90haa1d8f` opens 9443/tcp ingress publicly.

## Accessing the web panel

From Yuriy's Mac:

```bash
ssh -fN -L 2053:127.0.0.1:2053 yc-vm
open http://localhost:2053/
```

Login: `admin` / password from `~/.config/3x-ui-bridge/admin-password.txt`.

If Yuriy is not at his Mac, anyone can SSH-tunnel from any machine that has
his SSH key. The panel is bound to `127.0.0.1` on bridge — never reachable
from public internet.

## Telegram bot

`@ggg_3x-ui_bot` is configured in the panel (`Settings → Telegram bot`).
Token + admin chat id (775260) live in `/opt/3x-ui/db/x-ui.db`. The bot
reaches `api.telegram.org` via a SOCKS5 proxy at `127.0.0.1:8119` (running
on the production xray). On a fresh deploy, before the bot can DM Yuriy,
he must `/start` the bot once.

## Health checks

```bash
# Container alive?
ssh yc-vm 'sudo docker compose -f /opt/3x-ui/docker-compose.yml ps'

# Listeners on bridge:
ssh yc-vm 'sudo ss -tlnp | grep -E ":(2053|2096|9443|8119)\b"'
# Expected (4 lines): 127.0.0.1:2053, 127.0.0.1:2096, *:9443, 127.0.0.1:8119

# Bot polling? (look for "Telegram bot receiver started", no proxy errors)
ssh yc-vm 'sudo docker logs 3x-ui --tail 30 2>&1 | grep -iE "tg|bot|proxy|error" | tail -10'

# 9443 reachable from internet?
nc -z -w 3 81.26.189.136 9443 && echo OK
```

## Adding/removing users

Through the web UI: Inbounds → click `test-3xui` → "+" Client → enter email,
optionally GB cap or expiry → Save → "QR" / "Get info" → copy the share URL.

Or via REST API (need session cookie from `/login`):

```bash
COOKIE=/tmp/3xui-cookie
curl -sS -c "$COOKIE" -X POST 'http://localhost:2053/login' \
  -d "username=admin&password=$ADMIN_PASS"

# Add a client to existing inbound (id=2 in our setup):
curl -sS -b "$COOKIE" -X POST 'http://localhost:2053/panel/api/inbounds/addClient' \
  -H 'Content-Type: application/json' \
  --data '{"id":2,"settings":"{\"clients\":[{...}]}"}'
```

(The SSH tunnel from previous section must be active.)

## Production xray's `bot-socks` inbound

On `yc-vm`, the production xray now has an extra inbound:

```jsonc
{
  "tag": "bot-socks",
  "port": 8119,
  "listen": "127.0.0.1",
  "protocol": "socks",
  "settings": {"udp": false, "auth": "noauth"}
}
```

It exists solely so 3x-ui's Telegram bot can reach api.telegram.org through
our existing foreign-egress tunnel. Default outbound (foreign-egress is first
in the list) is used implicitly. **If you change xray config and accidentally
drop this inbound, 3x-ui's bot will stop responding to Telegram commands.**

If lost, re-add (live, no restart):

```bash
ssh yc-vm 'sudo bash -c "cat > /tmp/socks-inbound.json" <<EOF
{"inbounds":[{"tag":"bot-socks","port":8119,"listen":"127.0.0.1","protocol":"socks","settings":{"udp":false,"auth":"noauth"}}]}
EOF
sudo /usr/local/bin/xray api adi -s 127.0.0.1:8080 < /tmp/socks-inbound.json'
```

Plus persist to `config.json` so it survives next restart (jq edit similar to
the live-add above).

## Rollback (kill the test stack)

```bash
ssh yc-vm 'cd /opt/3x-ui && sudo docker compose down -v'
~/yandex-cloud/bin/yc vpc security-group update-rules enpm15cfu8f90haa1d8f --remove-rule '<rule-id-9443>'
ssh yc-vm 'sudo /usr/local/bin/xray api rmi -s 127.0.0.1:8080 < <(echo "{\"tag\":\"bot-socks\"}")'
```

To also wipe panel state: `sudo rm -rf /opt/3x-ui` on bridge.

## A/B test interpretation

- Compare per-user gap pattern (10-30 min disconnects) on prod inbound (:443) vs test inbound (:9443) over 24-48h.
- If test stable + prod flakes → 3x-ui's xhttp/Reality defaults are better; copy them into our `config.json` and tear down the test stack.
- If both flake → problem isn't in transport config, look at network path / clients.
- If prod stable + test flakes → our minimal config is fine; 3x-ui's defaults are worse.

Bridge log to watch: `/var/log/xray/access.log` (production). Test inbound
stats are visible in 3x-ui panel under Inbounds → Per-Client traffic.
