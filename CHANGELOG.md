# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/gaiverrr/amnezia-xray-admin/compare/v0.1.7...HEAD
[0.1.7]: https://github.com/gaiverrr/amnezia-xray-admin/compare/v0.1.0...v0.1.7
[0.1.0]: https://github.com/gaiverrr/amnezia-xray-admin/releases/tag/v0.1.0
