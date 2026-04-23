---
name: amnezia-ops
description: Operate Yuriy's personal double-hop Xray VPN (RU bridge on yc-vm 81.26.190.206 + foreign egress on vps-vpn 103.231.72.109). Use when the user says "VPN не работает", "проверь VPN", "кто-то не может подключиться", "добавь/удали юзера", "сгенери URL/QR", "обнови сертификат", "смени SNI", "мигрируй на другой VPS", "потерял bridge/egress", or mentions specific users (Masha, ivan, rita, Tima, Anton, Alexander, Anna, Kostya, Natasha, sanek, router, yuriy, Admin) having connection issues. Covers health diagnosis, user CRUD on the new double-hop bridge (native xray at /usr/local/etc/xray/config.json) AND the legacy Amnezia container (managed by Telegram bot), Let's Encrypt renewal, Reality key rotation, VPS migration, and disaster recovery. NOT for Rust development of the amnezia-xray-admin codebase — this skill is for live ops only.
---

# Amnezia Ops

Operational runbook for Yuriy's double-hop Xray VPN. When this skill fires, your job is to **diagnose and fix live issues over SSH**, not to write Rust. Prefer inspect-then-act over assumptions.

## Infrastructure at a glance

| Role | SSH alias | Public | Config |
|---|---|---|---|
| Bridge (clients connect here) | `yc-vm` | `81.26.190.206:443` | `/usr/local/etc/xray/config.json` (native xray, XHTTP+Reality, SNI `www.sberbank.ru`) |
| Egress (bridge → internet) | `vps-vpn` | `103.231.72.109:8444` | `/usr/local/etc/xray/config.json` (native xray, dest `127.0.0.1:9443` = local nginx with LE cert for `yuriy-vps.duckdns.org`) |

Logs: `/var/log/xray/{access,error}.log` on both. Restart: `sudo systemctl restart xray`.

Legacy services on `vps-vpn` — **do not touch unless explicitly asked**:
- Amnezia Docker VPN on `:443` (container `amnezia-xray`) — still serves ~14 old-format users.
- `mtproxymax` Telegram proxy on `:8443`.

Telegram bot process on `vps-vpn`: `amnezia-xray-admin --telegram-bot --local ...` managing the **legacy Amnezia**. A bridge-aware version is planned but not yet deployed.

For the full inventory (key files, ports, sidecar state, backup locations), read `references/inventory.md`.

## Routing: pick ONE reference based on user intent

Do not read every file — progressive disclosure.

| Request | Read | First action |
|---|---|---|
| "проверь VPN" / "что-то сломалось" / "все отвалились" | `references/health-check.md` | Run `scripts/health.sh` — safe read-only check of both hops. |
| One specific user can't connect | `references/troubleshooting.md` § User-specific | `ssh yc-vm 'sudo grep <email>@vpn /var/log/xray/access.log \| tail -20'`. If empty → their TCP never reached us. If REJECT entries → Reality handshake fail. |
| Add / remove / regenerate URL | `references/user-ops.md` | New bridge: `scripts/bridge-user-add.sh <name>` / `scripts/bridge-url.sh <name>`. Legacy: Telegram bot or ssh to vps-vpn and run the binary. |
| LE cert issue / certbot renewal | `references/troubleshooting.md` § Cert | `ssh vps-vpn 'sudo certbot certificates'`. Port 80 must be free — check `ss -tlnp \| grep :80`. |
| "sberbank.ru больше не палит / сменить SNI" | `references/troubleshooting.md` § SNI rotation | Edit `inbounds[0].streamSettings.realitySettings.{dest,serverNames}` on bridge, restart. Good fallbacks: `www.gosuslugi.ru`, `www.wildberries.ru` (verified TLS 1.3 + H2 from yc-vm). |
| Rotate Reality keys (paranoia or suspected compromise) | `references/user-ops.md` § Reality rotation | Regenerate x25519 on one hop → update the other → regenerate ALL client URLs. |
| "Мигрируй на новый VPS" (bridge OR egress) | `references/migration.md` | Read the full runbook first — bridge and egress flows differ. |
| "Сдох VPS, поднимаем с нуля" | `references/disaster-recovery.md` | Confirm which hop died before running anything. |
| Something not above | Ask the user one clarifying question. |

## Invariants (never violate)

1. **Never touch legacy Amnezia Docker or `mtproxymax` on `vps-vpn`** unless the user says to. They live on :443 and :8443 respectively.
2. **Backup `config.json` before every edit:** `sudo cp /usr/local/etc/xray/config.json /usr/local/etc/xray/config.json.bak-$(date +%s)`.
3. **Restart xray once after config change, then tail `error.log` ≥2 seconds** to catch startup failures. If non-empty after "started" line → revert.
4. **One hop at a time.** Never restart bridge and egress simultaneously.
5. **Destructive or user-visible actions require confirmation** unless the user said "yes" / "просто сделай".
6. **The Telegram bot currently manages the legacy Amnezia only.** Do not promise users that `/add` on the bot affects the new bridge until the bridge-aware bot ships.

## If something breaks mid-operation

Stop and report exactly what you ran, what the output was, and the current state of both hops. Do not guess-fix. Rollback hints live at the end of each reference file.

## Helper scripts

Under `scripts/` in this skill. They SSH from your laptop using aliases `yc-vm` and `vps-vpn`. Invoke from any CWD.

- `health.sh` — read-only health check of both hops (xray + nginx + LE cert expiry).
- `bridge-user-add.sh <name>` — add user to bridge config.json, restart xray, print vless URL + ASCII QR.
- `bridge-user-remove.sh <name>` — remove user by email prefix.
- `bridge-url.sh <name>` — regenerate the vless URL for an existing user (read-only, no restart).

All scripts bail out with a clear error if the required SSH alias is missing.
