# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Scope (as of 2026-04-23, v0.4.0)

This is **Yuriy's personal Xray VPN admin tool**, not a product.
Current functionality:

- CLI: `--list-users`, `--add-user`, `--delete-user`, `--user-url`,
  `--user-qr`, `--online-status`, `--server-info`.
- Telegram bot (`--telegram-bot`): `/users`, `/add`, `/delete`,
  `/url`, `/qr`, `/status`.
- Always talks to a native-systemd xray on the bridge host
  (`/usr/local/etc/xray/config.json`). No Amnezia-Docker support.
- Shipped via Homebrew formula in `gaiverrr/homebrew-tap`.

**Out of scope:** TUI (dropped in v0.4.0), Amnezia-Docker admin,
VPS migration subcommand (handled by `.claude/skills/amnezia-ops/`),
MTProxy management.

## Build & Test Commands

```bash
cargo build                    # dev build
cargo build --release          # release build
cargo test                     # run full test suite
cargo test xray::client        # run tests in one module
cargo clippy -- -D warnings    # lint (warnings are errors)
cargo fmt --check              # format check
cargo run -- --help            # print CLI help
```

### CLI commands (non-interactive)

```bash
cargo run -- --host yc-vm --list-users             # list users on bridge
cargo run -- --host yc-vm --user-url <name>        # print vless:// URL
cargo run -- --host yc-vm --user-qr <name>         # render ASCII QR in terminal
cargo run -- --host yc-vm --server-info            # bridge params + user count + xray version
cargo run -- --host yc-vm --online-status          # list configured users (native bridge has no online API)
cargo run -- --host yc-vm --add-user <name>        # add user, reload xray, print URL + QR
cargo run -- --host yc-vm --delete-user <name> --yes
cargo run -- --local --host <bridge-ip> --list-users   # run on bridge host (no SSH)
cargo run -- --telegram-bot --local --admin-id <ID> --host <bridge-ip>  # run the bot
```

`--host` accepts either an SSH alias from `~/.ssh/config` or a literal
hostname/IP. `connect_ssh` expands aliases via `resolve_ssh_host` and stores
the resolved hostname on the backend — that's what ends up in generated URLs.

## Architecture

**Two execution modes**: CLI (one-shot commands), Telegram bot (long-running daemon on the bridge).

**Backend abstraction** (`XrayBackend` trait) lets both modes share the xray ops code:

```
                    ┌───────────────┐
                    │  XrayClient   │  (list/add/remove users, bridge_public_params, reload_xray)
                    └───────┬───────┘
                            │ uses
                    ┌───────▼───────┐
                    │  XrayBackend  │  (trait: exec_on_host, exec_in_container)
                    └───┬───────┬───┘
           ┌────────────▼┐    ┌─▼────────────┐
           │  SshBackend │    │ LocalBackend │
           │  (remote)   │    │ (on bridge)  │
           └─────────────┘    └──────────────┘
           Used by:            Used by:
           - CLI (default)     - CLI --local
           - Bot (rare)        - Telegram bot (typical)
```

Both backends are thin native-shell wrappers — no `docker exec` wrapping anywhere.
`exec_in_container` exists for compatibility with the trait but just delegates
to `exec_on_host` for the native bridge.

## Module Responsibilities

- **config.rs** — TOML config at `~/.config/amnezia-xray-admin/config.toml`, clap `Cli` struct, merge logic.
- **ssh.rs** — Pure-Rust SSH via russh. Parses `~/.ssh/config` for alias resolution, TOFU known_hosts, `exec_command`.
- **backend_trait.rs** — `XrayBackend` trait + `SshBackend` + `LocalBackend`.
- **xray/client.rs** — `XrayClient`: `list_clients`, `add_client`, `remove_client`, `get_uuid`, `bridge_public_params`, `reload_xray`. Mutations do **not** restart xray; the caller invokes `reload_xray()` separately (see "bot proxies through xray" note below).
- **xray/config_render.rs** — `parse_bridge_config` + `ClientEntry` struct for reading `/usr/local/etc/xray/config.json`.
- **xray/url.rs** — `render_xhttp_url`, `render_qr_png`, `render_qr_ascii` for XHTTP+Reality `vless://` URLs.
- **xray/types.rs** — `XrayUser`, `TrafficStats`.
- **telegram.rs** — teloxide bot: `/users`, `/status`, `/add`, `/delete`, `/url`, `/qr`. Inline keyboard for buttons. Admin ID from `--admin-id` / `ADMIN_ID`. `/add` sends both URL text and QR photo by default.
- **migrate/install.rs** — unit-tested provisioning helpers (`apt_install`, `install_xray`, `preflight`, `generate_secrets`, `write_xray_config`, `restart_xray`). Not wired into a CLI subcommand — kept for potential future skill-driven use.
- **error.rs** — `AppError` + `Result<T>` + `add_hint()` for actionable error messages.
- **main.rs** — CLI parsing, dispatch, `connect` / `connect_ssh` / `connect_local` helpers.

## Important Design Details

- **Shell safety**: user names can contain brackets/spaces (e.g., `Admin [macOS Tahoe]`). `client.rs` uses `validate_name` + JSON encoding rather than shell-level quoting.
- **Email format**: `name@vpn` — derived from config entry name, used as the unique identifier inside `config.json`.
- **Reality public-key sidecar**: `/usr/local/etc/xray/reality-public-key` (one-line file) is the source of truth for `bridge_public_params`. It is written once at bridge setup time so URL generation doesn't have to re-derive the public key from the private key every call.
- **Bot uses xray as its own HTTP proxy**: the Telegram bot on the RU bridge routes `api.telegram.org` traffic through a local xray HTTP-proxy inbound on `127.0.0.1:8118` (itself routed out via `foreign-egress`) because RU ISPs filter the Bot API. That means `add_client` / `remove_client` must NOT restart xray — the `telegram.rs` handlers call `reload_xray()` **after** they finish sending responses. Otherwise the reload kills the proxy mid-response.
- **Bot needs `--host <public-ip>`**: without it, `LocalBackend::hostname()` returns whatever was passed (or `"localhost"`) and that ends up in generated URLs. The systemd unit's `ExecStart` always passes the current bridge IP explicitly.
- **Alias resolution**: CLI URL generation uses `backend.hostname()`, which `connect_ssh` sets to the **resolved** hostname (after expanding `~/.ssh/config` aliases). Do not use `cli.host` / `config.host` for URL rendering — those are pre-resolution.

## Live infrastructure (as of 2026-04-24)

- **Bridge** (ssh alias `yc-vm`, `81.26.189.136`, Yandex Cloud ru-central1-d): native xray on :443 (XHTTP+Reality, SNI `www.sberbank.ru`). Routes `geoip:ru → direct`, else → `foreign-egress`. Hosts the VPN management Telegram bot as `amnezia-xray-bot.service`. Also carries a `dokodemo-door` inbound on :8443 (`mtproxy-in → direct`) that TCP-forwards to `vps-vpn:8443` — so MTProxy clients reach it via the RU IP.
- **Egress** (ssh alias `vps-vpn`, `103.231.72.109`, Stark): native xray on :8444 + nginx self-steal on `127.0.0.1:9443` with LE cert for `yuriy-vps.duckdns.org`. Outbound freedom. Also runs `mtproxymax` Docker container on :8443 (faketls with `cloudflare.com` SNI) and its `mtproxymax-telegram.service` bot — mtproxymax lives HERE, not on bridge, because Yandex Cloud IPs cannot reach Telegram DCs.
- **Legacy on `vps-vpn`, stopped**: old Amnezia Docker container (`amnezia-xray`, exited), files preserved for now. Users already migrated to the double-hop. Filed for removal in `amnezia-xray-admin-mgj`.
- **Runbook**: `.claude/skills/amnezia-ops/` — project-local skill for live VPN fixes (health check, user CRUD, key rotation, disaster recovery).

## Release Process

To publish a new release (e.g. v0.4.0):

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md` with a new version section
3. Commit: `git commit -am "chore: release v0.4.0"`
4. Tag: `git tag -a v0.4.0 -m "v0.4.0"`
5. Push: `git push origin main --tags`
6. GitHub Actions `release.yml` builds binaries for linux-x86_64, linux-aarch64, macos-x86_64, macos-aarch64 and creates a GitHub Release.
7. `scripts/release.sh` automates steps 1–5 and updates the Homebrew formula in `gaiverrr/homebrew-tap`.

CI runs on pushes to main and on all pull requests via `.github/workflows/ci.yml` (test + clippy + fmt on ubuntu + macos).
