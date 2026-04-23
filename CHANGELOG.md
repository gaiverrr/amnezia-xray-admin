# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-04-23

### Added
- `--bridge` flag for `--telegram-bot` mode. Routes bot commands through a
  new `NativeXrayClient` that talks to native xray at
  `/usr/local/etc/xray/config.json` (no Amnezia Docker wrapping). Use on
  the double-hop bridge (yc-vm).
- `/add` (Telegram bot) now replies with both the vless URL text and a QR
  code photo by default.
- CLI `--add-user <name>` now prints the URL and an ASCII QR after it,
  instead of just the URL.
- New `src/native/` module: `NativeLocalBackend`, `NativeSshBackend`,
  `NativeXrayClient` (list/add/remove/get_uuid/bridge_public_params).
- New `src/migrate/install.rs` unit-tested helpers for VPS provisioning
  primitives: `apt_install`, `install_xray`, `preflight`, `generate_secrets`,
  `write_xray_config`, `restart_xray`.
- Snapshot tests for bridge/egress config rendering and XHTTP vless URL.
- Project-local operations skill at `.claude/skills/amnezia-ops/` with
  SKILL.md + references + `health.sh` helper.

### Changed
- Internal: new `src/migrate/` module scaffolded for future VPS-migration
  subcommands. The subcommand dispatch itself is deferred — the
  `--migrate-bridge` / `--migrate-egress` flags are reserved but not wired.

## [0.2.1] - 2026-04-20

### Fixed

- `--upgrade-xray` now follows redirects when fetching `.dgst` checksums (GitHub Releases returns a 302 to the CDN; without `-L` the body was empty and SHA256 verification always failed)
- `--upgrade-xray` now accepts `SHA2-256=` in `.dgst` files — the OpenSSL format XTLS actually ships, not `SHA256=`

## [0.2.0] - 2026-04-20

### Added

- `--snapshot` / `--snapshot-list` / `--snapshot-restore [tag]` CLI commands for point-in-time backups of server config + Xray binary (stored on host FS via `docker cp`)
- `--upgrade-xray` CLI command: safe binary upgrade with SHA256 verification, host arch auto-detection (x86_64 / aarch64 / armv7), pre-upgrade snapshot, and auto-rollback on failure
- Telegram bot: `/snapshot`, `/snapshots`, `/restore`, `/upgrade` admin commands (with inline-keyboard confirmation for destructive actions)
- Telegram bot: `/route`, `/unroute`, `/routes` commands for managing direct-access routing rules
- Telegram bot: commands are now scoped — non-admin users only see `/start` and `/help` in the `/` menu (`BotCommandScopeChat`)
- Container uptime + latest Xray version indicator in dashboard status bar, `--server-info`, and Telegram `/status`
- Configurable `bot_image` in `config.toml` for `--deploy-bot`
- Configurable `snapshot_dir` in `config.toml` / `--snapshot-dir` flag

### Changed

- Dashboard caches latest Xray version once per session (no longer hits GitHub API on every 5 s refresh)
- Version-check response parsed via `serde_json` and validated against `[0-9.]+` before display (no longer trusts remote shell pipeline output)
- Centralized `fetch_container_uptime` / `fetch_latest_xray_version` helpers — single implementation shared by TUI, CLI, and Telegram bot
- `docker ps --filter name=X` is now anchored (`name=^/X$`) to prevent substring matches

### Fixed

- Clippy `collapsible_match` warning under Rust 1.95 (`src/ssh.rs`)

### Security

- SHA256 checksum verification of downloaded Xray binary before replacing inside container
- Snapshot tags validated as `YYYYMMDD-HHMMSS` to block path traversal via `/restore <tag>`
- `xray version` output is base64-encoded before interpolation into shell commands (defense against quote-breakout)

## [0.1.7] - 2026-03-22

### Fixed

- xray api adu: use full inbounds config format required by xray v25+ — new users now connect successfully
- xray api rmu: use `-tag=vless-in email` syntax instead of deprecated `-email` flag
- UNKNOWN_IP in vless:// URLs when bot runs in Docker — deploy now passes VPS public IP via `--host`
- Docker image: switch to musl static binary + alpine (no glibc dependency, ~8MB image)
- Deploy no longer compiles Rust on VPS — pulls pre-built image from ghcr.io (~30 seconds)
- ARG_MAX error when uploading large files during deploy

### Added

- Telegram bot: inline keyboard buttons for `/url`, `/qr`, `/delete` when called without argument
- Telegram bot: `/delete` confirmation step with Yes/Cancel buttons
- Telegram bot: `/add` without argument shows usage hint
- Telegram bot: `--admin-id` flag for secure admin authentication (replaces first-/start auto-detect)
- TUI Telegram setup screen: admin ID input field with @userinfobot hint
- Deploy progress indicator with step-by-step status and spinner animation
- Auto-backup of server.json and clientsTable before every mutation (add, delete, rename, API setup)
- CLI command `--backup` for creating timestamped backups
- CLI command `--restore [timestamp]` for restoring from backups
- CLI command `--add-user <name>` for adding users non-interactively
- CLI command `--delete-user <name>` with interactive confirmation (use `--yes` to skip)
- CLI command `--rename-user <old> <new>` for renaming VPN users
- Categorized usage examples in `--help` output
- Actionable error messages with troubleshooting hints for SSH, container, and API errors
- GitHub Actions release workflow publishes Docker image to ghcr.io

### Changed

- Improved TUI layout: better column alignment, polished spacing in dashboard, user detail, and setup screens

## [0.1.0] - 2026-03-21

### Added

- TUI dashboard with hacker-aesthetic theme for managing Amnezia VPN's Xray server
- First-run setup wizard for SSH connection and container configuration
- User management: add, delete, and view VLESS users
- Per-user traffic statistics (upload/download) via xray stats API
- Real-time online status showing connected users and their IPs
- QR code generation for sharing VLESS connection URLs
- User detail view with connection URL and QR code
- Auto-refresh every 5 seconds to keep dashboard data current
- CLI commands for non-interactive use:
  - `--list-users` — list users with traffic stats
  - `--check-server` — verify API setup and print xray version
  - `--user-url <name>` — print VLESS URL for a user
  - `--user-qr <name>` — render QR code in terminal
  - `--online-status` — show connected users and IPs
  - `--server-info` — xray version, traffic summary, user count
- SSH backend using pure-Rust `russh` with `~/.ssh/config` support and TOFU host key verification
- Local backend for running directly on VPS without SSH
- Automatic xray server configuration (API, stats, policy sections)
- TOML configuration file at `~/.config/amnezia-xray-admin/config.toml`
- Telegram bot mode with commands: /start, /help, /users, /status, /add, /delete, /url, /qr
- Bot deployment command (`--deploy-bot`) to set up the Telegram bot on VPS via SSH

[Unreleased]: https://github.com/gaiverrr/amnezia-xray-admin/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/gaiverrr/amnezia-xray-admin/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/gaiverrr/amnezia-xray-admin/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/gaiverrr/amnezia-xray-admin/compare/v0.1.7...v0.2.0
[0.1.7]: https://github.com/gaiverrr/amnezia-xray-admin/compare/v0.1.0...v0.1.7
[0.1.0]: https://github.com/gaiverrr/amnezia-xray-admin/releases/tag/v0.1.0
