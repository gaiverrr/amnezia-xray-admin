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

## Modifying the xray template config (outbounds, routing rules)

3x-ui stores its xray template in sqlite under `key='xrayTemplateConfig'`,
but **the HTTP API field name is `xraySetting`** (form-encoded). No
partial-update — always send the full template. User inbounds live in a
separate `inbounds` DB table and are merged in at xray-launch time; do not
include them in the template.

### Read current template

```bash
curl -sS -b "$COOKIE" -X POST 'http://localhost:2053/panel/xray/' | \
  python3 -c 'import json,sys;d=json.load(sys.stdin)["obj"];print(d["xraySetting"])' \
  > /tmp/tpl.json
# obj.xraySetting is a JSON-encoded STRING — needs json.loads to mutate.
# obj.inboundTags is an array (auto-named "inbound-<port>" or "inbound-<listen>:<port>").
```

### Write template

```bash
curl -sS -b "$COOKIE" -X POST 'http://localhost:2053/panel/xray/update' \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode "xraySetting=$(cat /tmp/tpl.json)" \
  --data-urlencode "outboundTestUrl=https://www.google.com/generate_204"
# Validates via xray.Config unmarshal. Rejects with "xray template config
# invalid: ..." if anything's off.
```

### Apply

`/panel/xray/update` does NOT auto-restart xray. After save:

```bash
curl -sS -b "$COOKIE" -X POST 'http://localhost:2053/panel/setting/restartPanel'
```

### Inbound tag naming (hardcoded, NOT user-configurable via API)

- `listen` empty / `0.0.0.0` / `::`           → tag = `inbound-<port>`
- specific listen (e.g. `127.0.0.1`)          → tag = `inbound-<listen>:<port>`

Source: `web/controller/inbound.go:113-117`, `web/service/inbound.go:495-500`.

### Workflow: add an outbound + routing rule (e.g. double-hop egress)

```bash
# Fetch current template
curl -sS -b "$COOKIE" -X POST 'http://localhost:2053/panel/xray/' \
  | python3 -c 'import json,sys;print(json.load(sys.stdin)["obj"]["xraySetting"])' \
  > /tmp/tpl-current.json

# Mutate
python3 << 'PY' > /tmp/tpl-new.json
import json
t = json.load(open('/tmp/tpl-current.json'))
t['outbounds'].append({...new outbound dict...})
t['routing']['rules'].append({"type":"field","inboundTag":["inbound-9443"],"outboundTag":"foreign-egress"})
print(json.dumps(t))
PY

# Push + apply
curl -sS -b "$COOKIE" -X POST 'http://localhost:2053/panel/xray/update' \
  --data-urlencode "xraySetting=$(cat /tmp/tpl-new.json)" \
  --data-urlencode "outboundTestUrl=https://www.google.com/generate_204"
curl -sS -b "$COOKIE" -X POST 'http://localhost:2053/panel/setting/restartPanel'

# Verify live config picked up the change
ssh yc-vm 'sudo docker exec 3x-ui cat bin/config.json' \
  | jq '{outbounds:[.outbounds[].tag], rules:[.routing.rules[]|{inboundTag,outboundTag}]}'
```

### Sources

- `web/controller/xray_setting.go` (lines 32–99: routes, GET, update)
- `web/service/xray_setting.go` (lines 17–89: Save + UnwrapXrayTemplateConfig)
- `web/service/inbound.go` (lines 1585–1594: GetInboundTags)
- `database/model/model.go` (lines 48–112: Inbound + Setting schemas)
- Postman collection (only quasi-official): https://www.postman.com/hsanaei/3x-ui/collection/q1l5l0u/3x-ui

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
