# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # dev build
cargo build --release          # release build
cargo test                     # all 443 tests
cargo test xray::client        # run tests in one module
cargo test test_build_rmu      # run a single test by name
cargo clippy                   # lint (expect dead_code warnings in telegram module)
cargo fmt --check              # format check
cargo run                      # launch TUI
cargo run -- --ssh-host vps-vpn --container amnezia-xray  # with CLI args
```

### CLI commands (non-interactive)

```bash
cargo run -- --list-users                          # list users with traffic stats
cargo run -- --check-server                        # verify API setup, print xray version
cargo run -- --user-url <name>                     # print vless:// URL for a user
cargo run -- --user-qr <name>                      # render QR code in terminal
cargo run -- --online-status                       # show online users and IPs
cargo run -- --server-info                         # xray version, traffic, user count
cargo run -- --local --list-users                  # use local docker exec (on VPS)
cargo run -- --add-user <name>                     # add user, print URL
cargo run -- --delete-user <name> --yes             # delete user (--yes skips confirmation)
cargo run -- --rename-user <old> <new>              # rename user (resets traffic stats)
cargo run -- --backup                               # create timestamped backup
cargo run -- --restore                              # restore latest backup
cargo run -- --restore <timestamp>                  # restore specific backup (YYYYMMDD-HHMMSS)
cargo run -- --telegram-bot --local --container c  # run Telegram bot daemon
cargo run -- --deploy-bot --telegram-token <TOKEN> --admin-id <ID>  # deploy bot to VPS via SSH
```

### Bridge mode (new double-hop setup)

For the new RU-bridge + foreign-egress setup, pass `--bridge` to opt into the
native-xray code path instead of Amnezia Docker:

```bash
# Telegram bot against new bridge (deploy binary to bridge host, run locally)
amnezia-xray-admin --telegram-bot --local --bridge --admin-id <ID>

# Add user directly on bridge via CLI (prints URL + ASCII QR)
amnezia-xray-admin --bridge --local --add-user <name>
```

In `--bridge` mode, the tool reads/writes `/usr/local/etc/xray/config.json`
directly and does not use `docker exec`. The Reality public key must be
stored at `/usr/local/etc/xray/reality-public-key` (one-line file) at
bridge setup time so `bridge_public_params` can render URLs without
re-running `xray x25519`.

Commands `/snapshot`, `/restore`, `/upgrade`, `/routes` are not supported
in bridge mode yet — the bot will reply "not supported in bridge mode".

### Deploy prerequisites

Bot deploy cross-compiles locally for Linux, then uploads the binary to VPS. Requires `cross`:
```bash
cargo install cross
```
```

## Architecture

**Three execution modes**: TUI (interactive dashboard), CLI (one-shot commands), Telegram bot (daemon on VPS).

**Backend abstraction** (`XrayBackend` trait) enables all modes to share xray operation code:
```
                    ┌─────────────────┐
                    │  XrayApiClient  │  (list/add/remove users, stats, online)
                    └────────┬────────┘
                             │ uses
                    ┌────────▼────────┐
                    │  XrayBackend    │  (trait: exec_in_container, exec_on_host)
                    └────┬───────┬────┘
              ┌──────────▼┐  ┌──▼──────────┐
              │ SshBackend │  │LocalBackend │
              │ (remote)   │  │(on VPS)     │
              └────────────┘  └─────────────┘
              Used by:          Used by:
              - TUI              - Telegram bot (--local)
              - CLI (remote)     - CLI --local
```

**Async TUI pattern**: Synchronous ratatui event loop + tokio runtime for background operations. Communication via `mpsc` channel (`BackendMsg` enum).

**Key flow**: TUI calls `backend::spawn_*()` → tokio task runs SSH commands → sends `BackendMsg` back → `App::process_backend_messages()` updates state → next `draw()` renders.

**Guard flags** (`pending_refresh`, `pending_add_name`, etc.) prevent duplicate async operations. `refresh_after_mutation` handles stale fetch results after add/delete.

**Telegram bot**: Uses `teloxide` framework. Runs as `--telegram-bot` mode with `LocalBackend` on VPS. Admin ID set at deploy time via `--admin-id`. Commands: /users, /status, /add, /delete, /url, /qr with inline keyboard buttons.

**Deploy strategy**: Cross-compiles binary locally (Mac/Linux → linux-x86_64/aarch64 via `cross`), uploads pre-built binary to VPS, wraps in minimal Docker image (debian:slim + docker.io). No Rust toolchain needed on VPS. Auto-detects VPS architecture.

## Module Responsibilities

- **config.rs**: TOML config at `~/.config/amnezia-xray-admin/config.toml`, clap CLI args, merge logic
- **ssh.rs**: Pure-Rust SSH via russh. Parses `~/.ssh/config`, TOFU known_hosts verification, `exec_command()` / `exec_in_container()`
- **backend_trait.rs**: `XrayBackend` trait + `SshBackend` and `LocalBackend` implementations
- **xray/types.rs**: Data types (`XrayUser`, `ServerConfig`, `ClientsTable`). `ServerConfig` wraps `serde_json::Value` to preserve unknown fields
- **xray/config.rs**: `ensure_api_enabled()` — one-time server.json transformation (adds stats/policy/api sections, emails, level:0)
- **xray/client.rs**: `XrayApiClient` — list/add/remove/rename users, stats, online status, backup/restore. Commands run via `docker exec <container> xray api ...`
- **backend.rs**: Async task spawners, `BackendMsg` enum, connection helpers
- **telegram.rs**: Telegram bot module using teloxide. Commands: /start, /help, /users, /status, /add, /delete, /url, /qr. Inline keyboard buttons for /url, /qr, /delete without argument. Callback query handler for button actions. Admin ID from `--admin-id` / `ADMIN_ID` env var (no auto-detect). With `--bridge`, every command handler branches to `NativeXrayClient` instead of `XrayApiClient`. `/add` sends URL text **and** QR photo by default.
- **native/** (bridge/egress with native systemd xray, no Docker): `backend.rs` has `NativeSshBackend` and `NativeLocalBackend` (XrayBackend impls without docker-exec wrapping). `client.rs` has `NativeXrayClient` with `list_clients`, `add_client`, `remove_client`, `get_uuid`, `bridge_public_params`, `reload_xray`. `config_render.rs` has pure-function `render_bridge_config` / `render_egress_config` + `parse_bridge_config`. `url.rs` renders XHTTP+Reality `vless://` URLs and QR codes (PNG + ASCII).
- **migrate/** (scaffolded, not wired into CLI): `install.rs` has unit-tested helpers (`apt_install`, `install_xray`, `preflight`, `generate_secrets`, `write_xray_config`, `restart_xray`). `bridge.rs` / `egress.rs` are empty stubs — migrate-bridge/egress subcommands deferred in favour of the `.claude/skills/amnezia-ops/` runbook.
- **error.rs**: `AppError` enum (SSH, Xray, Config, IO variants), `Result<T>` type alias, and `add_hint()` which enriches error messages with actionable troubleshooting suggestions
- **app.rs**: 6-screen state machine (Setup→Dashboard→UserDetail/AddUser/QrView/TelegramSetup), event loop with 250ms poll + 5s auto-refresh
- **ui/**: TUI rendering submodules — setup.rs (wizard), dashboard.rs (main view), user_detail.rs (detail panel), add_user.rs (add dialog), qr.rs (QR display + CLI rendering), telegram_setup.rs (bot deploy screen), theme.rs (color constants)

## Important Design Details

- **No bind mounts**: Amnezia stores config files *inside* the container at `/opt/amnezia/xray/`. All file reads/writes must use `exec_in_container()`, not `exec_command()`.
- **Shell safety**: User names can contain brackets/spaces (e.g., "Admin [macOS Tahoe]"). Use `shell_quote()` for xray API commands, base64 for JSON payloads.
- **Stats require `level: 0`**: Xray only tracks per-user traffic when clients have `"level": 0` matching the policy section.
- **clientsTable format**: `userData` is an object `{"clientName": "...", "creationDate": "..."}`, not a plain string.
- **Email format**: `name@vpn` — derived from clientsTable name, used as xray stats identifier.
- **Auto-backup**: `backup_config()` runs before every mutation (add/remove/rename/ensure_api_enabled). Creates `.bak` copies of server.json and clientsTable inside the container.
- **Timestamped backups**: `backup_config_timestamped()` creates backups with `YYYYMMDD-HHMMSS` suffix for `--backup` CLI command.
- **Bridge mode = bot uses xray as HTTP proxy**: The `--bridge` bot on the RU bridge routes its own `api.telegram.org` traffic through a local xray HTTP-proxy inbound on `127.0.0.1:8118` (itself routed out via `foreign-egress`) because RU ISPs filter Telegram Bot API. Consequently `NativeXrayClient::{add_client,remove_client}` deliberately do NOT restart xray — the callers in `src/telegram.rs` invoke `reload_xray()` **after** all `bot.send_*` calls, otherwise the reload kills the proxy mid-response.
- **Bridge bot needs `--host <public-ip>`**: Without it, `NativeBackend::hostname()` returns the VM hostname (`bridge-ru`), which ends up in vless URLs — users' clients can't resolve it. Always pass `--host 81.26.189.136` (or current bridge IP) in the systemd unit's `ExecStart`.

## Live infrastructure (as of 2026-04-23)

- **Bridge** (ssh alias `yc-vm`, `81.26.189.136`, Yandex Cloud ru-central1-d): native xray on :443 (XHTTP+Reality, SNI `www.sberbank.ru`). Routes `geoip:ru → direct`, else → `foreign-egress`. Hosts the Telegram bot as `amnezia-xray-bot.service`.
- **Egress** (ssh alias `vps-vpn`, `103.231.72.109`, Stark): native xray on :8444 + nginx self-steal on `127.0.0.1:9443` with LE cert for `yuriy-vps.duckdns.org`. Outbound freedom.
- **Legacy on `vps-vpn`, do not touch**: Amnezia Docker VPN on :443 (original users), `mtproxymax` MTProto proxy on :8443.
- **Runbook**: `.claude/skills/amnezia-ops/` — project-local skill for live VPN fixes (health check, user CRUD, key rotation, disaster recovery).

## Release Process

To publish a new release (e.g. v0.2.0):

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md` with new version section
3. Commit: `git commit -am "chore: bump version to 0.2.0"`
4. Tag: `git tag -a v0.2.0 -m "v0.2.0"`
5. Push: `git push origin main --tags`
6. GitHub Actions `release.yml` automatically builds binaries for linux-x86_64, linux-aarch64, macos-x86_64, macos-aarch64 and creates a GitHub Release
7. Update Homebrew formula in `gaiverrr/homebrew-tap`:
   - Update `url` to new tag tarball
   - Update `sha256` (download tarball, run `shasum -a 256`)
   - Push to homebrew-tap repo

CI runs on pushes to main and on all pull requests via `.github/workflows/ci.yml` (test + clippy + fmt on ubuntu + macos).
