# Epic B + C: VPS migration & Telegram bot upgrade

- **Status**: design approved, ready for implementation plan
- **Date**: 2026-04-23
- **Beads**: `amnezia-xray-admin-b9t` (migration) + `amnezia-xray-admin-02t` (bot — merged)
- **Depends on**: Epic A (architecture snapshot) — for current-state reference

## Summary

Two stateless Rust subcommands, `migrate-bridge` and `migrate-egress`, that by
SSH install a full stack on a freshly-provisioned VPS, generate fresh
keys/UUIDs, read the live user list from the old host, and atomically cut
over. No persistent state outside the live VPSes themselves — if both old and
new are dead at once, we rebuild from operator memory.

The existing Telegram bot (`--telegram-bot` mode) is upgraded in the same
codebase to drive the new bridge architecture: same command surface as today
(`/users`, `/status`, `/add`, `/delete`, `/url`, `/qr`), but the backend is
bridge's native xray (not Amnezia Docker). `migrate-bridge` re-provisions the
bot on the new bridge as part of the same flow — one command, both jobs.
On user creation (`/add` or CLI `--add-user`) the response includes **both**
the vless URL string and an inline QR code by default.

## Motivation

Our double-hop setup uses two VPSes: a RU bridge (Yandex Cloud today) and a
foreign egress (Stark today). Both are fungible — we want to replace them
when a cheaper provider shows up, when an IP gets flagged, or when a
provider's ToS turns hostile. Manual migration is ~2 hours of copy-pasting
keys and editing configs. Target: <15 min with one command, no errors.

## Design decisions (one line each with rationale)

| # | Decision | Rationale |
|---|---|---|
| 1 | Both bridge and egress migrations supported | User changes both opportunistically |
| 2 | SSH-only, no cloud provider APIs | Operator-provisioned VPS; provider-agnostic; fewer secrets |
| 3 | Regenerate all keys every migration | No long-lived secrets to store; minimizes metadata footprint (we bypass blocks — less is better) |
| 4 | Bridge = runtime source of truth for user list | Single source, no file can drift; accepted risk: lose bridge = lose user list |
| 5 | Hard cutover (not parallel / not time-bounded) | Simplest, ≤10 min user outage, no dual-run bookkeeping |
| 6 | Extend existing Rust tool, not bash | Reuse SSH/config code; consistent with bot upgrade in same tool; better for TDD |
| 7 | Bot co-located with bridge host | Bridge holds the only source of truth (clients[]); co-located bot avoids cross-host RPC for every command |
| 8 | Bot re-provisioned by `migrate-bridge` | Bridge migration already installs xray; installing bot (native Rust binary) is one extra step with the same cross-compiled artefact |
| 9 | Native binary deploy, no Docker-in-Docker | Old `--deploy-bot` wrapped in Debian+docker image because Amnezia was Docker; new bridge runs native xray, so no wrapper needed |
| 10 | `/add` and `--add-user` return URL + QR by default | User ask; removes the extra `/qr` round-trip for new users |

## Non-goals (YAGNI)

- No state file, no `~/.config` cache — pass `--old-ssh` explicitly every time.
- No preservation of client UUIDs / Reality keys across migrations.
- No automatic SNI fallback — if sberbank.ru probe fails, abort and let operator decide.
- No end-to-end traffic probe via embedded xray client — smoke tests on listening ports are enough.
- No `--keep-old` flag — operator answers `N` to the cutover prompt if they want to skip.
- No separate modules for DuckDNS / URL / config-template — inline in bridge.rs/egress.rs.
- No integration tests against real VPS in CI — covered by live dry-runs during acceptance.
- No new bot commands beyond the existing ones. Just rewire the backend.
- No bot on egress. Co-location on bridge is the simplest model.
- No separate bot-only migration command. `migrate-bridge` handles it end-to-end.

## Architecture

Two new subcommands dispatched from `src/main.rs`:

```
amnezia-xray-admin migrate-bridge --new-ssh <alias> --old-ssh <alias>
                                  [--telegram-token <TOKEN> --admin-id <ID>]
                                  [--yes] [--dry-run]
amnezia-xray-admin migrate-egress --new-ssh <alias> --old-ssh <alias> --bridge-ssh <alias>
                                  [--duckdns-token <TOKEN>] [--yes] [--dry-run] [--skip-old]
```

Bot re-provisioning on `migrate-bridge` is opt-in via `--telegram-token` +
`--admin-id`. If omitted, migration still runs; no bot on new bridge.

Both share `src/migrate/install.rs` (apt + xray/nginx/certbot/bot installers)
and are otherwise independent.

### Source layout

```
src/migrate/
├── mod.rs        CLI dispatch, prompts, shared helpers
├── bridge.rs     Bridge migration flow (4 phases)
├── egress.rs     Egress migration flow (7 phases)
└── install.rs    apt install, xray installer, systemd wait helpers
```

Reused from existing code:
- `src/ssh.rs` for SSH operations.
- `src/xray/client.rs::user_url()` for URL generation.
- `src/xray/config.rs` for reading/parsing server.json.
- `src/error.rs::AppError` — no new variants needed.
- `src/telegram.rs` — existing bot module; updated to drive bridge-xray
  backend (see "Bot changes" below) rather than Amnezia-Docker `LocalBackend`.

**Bot changes (inside existing `src/telegram.rs`):**
- Switch backend from current `LocalBackend` (which does `docker exec amnezia-xray xray api ...`) to a new thin wrapper that runs `xray api ...` against the local socket on bridge (no Docker).
- `/add`: after user creation, reply with the vless URL string **and** an
  inline image message with the QR code. Same behaviour in CLI `--add-user`.
- Same command surface: `/users`, `/status`, `/add`, `/delete`, `/url`, `/qr`, inline keyboards — preserved.
- Config file remains the one at `/usr/local/etc/xray/config.json` on the bridge host (no Amnezia clientsTable, no base64 dance — just `xray api` + direct edits).

## Flow: migrate-bridge

```
Phase 1 — pre-flight (non-destructive, abortable anytime)
  • SSH new: sudo -n, OS Ubuntu 22+/24+, RAM ≥ 512 MB, port 443 free, apt reachable
  • SSH old: read /usr/local/etc/xray/config.json → extract clients[] + foreign-egress outbound
  • Validate old inbound: type=xhttp, security=reality; abort if not
  • Probe sberbank.ru from new host: H2=2, TLSv1.3; abort if not reachable

Phase 2 — provision new bridge (old still untouched)
  • apt-get update + install curl jq openssl ca-certificates
  • Run xray-install.sh; check version ≥ 26.3
  • Generate keys: x25519, shortId (hex 8 bytes), path (hex 6 bytes); UUIDs per user
  • Render /usr/local/etc/xray/config.json with:
      - inbound: VLESS + XHTTP + Reality, dest www.sberbank.ru:443
      - clients: user list from old bridge
      - outbounds: foreign-egress (copied verbatim), direct, block
      - routing: geoip:ru→direct, else→foreign-egress
  • systemctl restart xray
  • Smoke test: listening on :443, error.log has only "started"

Phase 3 — URL generation (local machine, not on server)
  • For each user: render vless://UUID@NEW_IP:443?type=xhttp&…&sni=www.sberbank.ru
  • Write ./urls-<YYYYMMDD-HHMMSS>.txt locally
  • Print to stdout; if TTY, also render QR codes

Phase 4 — cutover (point of no return)
  • Prompt: "URLs ready. Stop old bridge? [y/N]"  (skip if --yes)
  • SSH old: systemctl stop xray + systemctl disable xray (and stop bot if it was there)
  • Print "Done. VPS can be destroyed in provider UI."

Phase 5 — bot deploy on new bridge (only if --telegram-token + --admin-id)
  • cross-compile amnezia-xray-admin linux target (reuse existing cross setup)
  • scp binary to new bridge (/usr/local/bin/amnezia-xray-admin)
  • install systemd unit /etc/systemd/system/amnezia-xray-bot.service
    • ExecStart=amnezia-xray-admin --telegram-bot --local --admin-id <ID>
    • Environment=TELOXIDE_TOKEN=<TOKEN>
  • systemctl daemon-reload + enable + start
  • Smoke test: journalctl -u amnezia-xray-bot shows "Bot registered commands"
```

**Rollback**: in Phases 1–3 nothing on old is touched; abort is free. After
Phase 4, rolling back requires operator action (re-enable xray on old SSH).

## Flow: migrate-egress

```
Phase 1 — pre-flight
  • SSH new: sudo -n, OS, ports 80/8444/9443 free, apt reachable
  • SSH bridge: read current foreign-egress outbound (sanity)
  • DNS check: yuriy-vps.duckdns.org resolves (still on old IP — fine)

Phase 2 — provision new egress
  • apt-get install nginx certbot curl jq openssl ca-certificates
  • xray-install.sh
  • Generate keys: x25519 (egress), shortId, path, BRIDGE_UUID
  • Write nginx conf for 127.0.0.1:9443 ssl http2 (do NOT start yet — port 80
    must be free for certbot)

Phase 3 — DuckDNS switch-over (first point of no return)
  • If --duckdns-token given: HTTP GET duckdns.org/update?domains=...&ip=<NEW>
  • Else: prompt operator to update manually, wait Enter
  • Poll `dig +short yuriy-vps.duckdns.org` until it matches new IP
    (timeout 120 s; abort if not propagated)

Phase 4 — LE cert + nginx on new
  • certbot certonly --standalone -d yuriy-vps.duckdns.org --non-interactive
    --agree-tos --email gaiverrr@gmail.com
  • systemctl start nginx
  • Verify: curl --resolve yuriy-vps.duckdns.org:9443:127.0.0.1 https://... → 200

Phase 5 — xray-egress config + start
  • Write /usr/local/etc/xray/config.json (inbound :8444 XHTTP+Reality,
    dest 127.0.0.1:9443, freedom outbound)
  • systemctl restart xray; smoke test
  • From bridge host: nc -zv NEW_IP 8444 (reachability)

Phase 6 — atomic cutover on bridge (second point of no return)
  • Prompt: "Switch bridge to new egress? [y/N]"  (skip if --yes)
  • SSH bridge: patch outbound foreign-egress (new IP + new path + new pbk +
    new sid + new UUID), keep serverName = yuriy-vps.duckdns.org
  • systemctl restart xray on bridge
  • Verify: tail bridge error.log, no TLS rejects

Phase 7 — decommission old egress
  • Prompt unless --yes
  • SSH old: systemctl stop xray + stop nginx + disable both
  • Skip if --skip-old (old is dead / unreachable)
```

**User-facing downtime**: near-zero. Bridge outbound switch is atomic; client
connections to bridge stay open, subsequent egress-bound packets flow
through the new egress within a second.

## Testing

**Unit** (pure, no SSH/network):
- `bridge::render_config()` snapshot test against fixture.
- `egress::render_config()` snapshot test against fixture.
- `url::generate()` — part-by-part formatting from (UUID, IP, pbk, sid, path, sni).
- `url::qr()` — QR image bytes for a given URL deterministic length (PNG header check).
- Parse fixture `tests/fixtures/bridge-config.json` → extract clients[] + outbound.
- Bot: `/add` command handler returns both URL string and QR image (mock teloxide bot).

**Integration** (mock SSH backend, using existing `XrayBackend` trait):
- `install::check_preflight()` — correct command sequence; abort on failure.
- Bridge phases 1–3 sequence; phase 4 not invoked without confirmation.

**Not tested**: real apt install, real certbot, real cut-over. Covered by
live acceptance runs below.

## Acceptance criteria

1. `cargo test` green, new code ≥ 80% line coverage.
2. `cargo clippy` clean; `cargo fmt --check` clean.
3. `--dry-run` prints full plan with zero side effects.
4. Live bridge migration between two Yandex Cloud VPSes (same provider for
   simplest test): <15 min end-to-end, all users reconnect with new URLs.
5. Live egress migration to a fresh Hetzner/Contabo VPS: DuckDNS flip,
   certbot succeeds, bridge outbound updated, client traffic egresses from
   new IP.
6. After live bridge migration with `--telegram-token/--admin-id`, the bot
   is running on the new bridge; `/users`, `/status`, `/add`, `/delete`,
   `/url`, `/qr` all functional; `/add` replies with URL + QR image.
7. CLI `--add-user` prints URL and renders ASCII QR in the terminal.
8. `CHANGELOG.md` entry; `CLAUDE.md` documents the new subcommands.

## URL delivery after migration

`migrate-bridge` produces `./urls-<timestamp>.txt` locally and prints URLs
to stdout. Operator delivers them manually to users. Auto-push to users
via the bot at migration time is **out of scope for this spec** — it adds
a dependency on Telegram-chat-id-per-user mapping that we don't have.
That feature can be layered later on top of the text-file interface.

## Open questions

None at design time. Surface during implementation.
