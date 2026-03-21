# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # dev build
cargo build --release          # release build
cargo test                     # all 350 tests
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
cargo run -- --telegram-bot --local --container c  # run Telegram bot daemon
cargo run -- --deploy-bot --token <TOKEN>          # deploy bot to VPS via SSH
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

**Telegram bot**: Uses `teloxide` framework. Runs as `--telegram-bot` mode with `LocalBackend` on VPS. First `/start` sender becomes admin (auto-detect). Commands: /users, /status, /add, /delete, /url, /qr.

## Module Responsibilities

- **config.rs**: TOML config at `~/.config/amnezia-xray-admin/config.toml`, clap CLI args, merge logic
- **ssh.rs**: Pure-Rust SSH via russh. Parses `~/.ssh/config`, TOFU known_hosts verification, `exec_command()` / `exec_in_container()`
- **backend_trait.rs**: `XrayBackend` trait + `SshBackend` and `LocalBackend` implementations
- **xray/types.rs**: Data types (`XrayUser`, `ServerConfig`, `ClientsTable`). `ServerConfig` wraps `serde_json::Value` to preserve unknown fields
- **xray/config.rs**: `ensure_api_enabled()` — one-time server.json transformation (adds stats/policy/api sections, emails, level:0)
- **xray/client.rs**: `XrayApiClient` — list/add/remove users, stats, online status. Commands run via `docker exec <container> xray api ...`
- **backend.rs**: Async task spawners, `BackendMsg` enum, connection helpers
- **telegram.rs**: Telegram bot module using teloxide. Commands: /start, /help, /users, /status, /add, /delete, /url, /qr
- **app.rs**: 6-screen state machine (Setup→Dashboard→UserDetail/AddUser/QrView/TelegramSetup), event loop with 250ms poll + 5s auto-refresh

## Important Design Details

- **No bind mounts**: Amnezia stores config files *inside* the container at `/opt/amnezia/xray/`. All file reads/writes must use `exec_in_container()`, not `exec_command()`.
- **Shell safety**: User names can contain brackets/spaces (e.g., "Admin [macOS Tahoe]"). Use `shell_quote()` for xray API commands, base64 for JSON payloads.
- **Stats require `level: 0`**: Xray only tracks per-user traffic when clients have `"level": 0` matching the policy section.
- **clientsTable format**: `userData` is an object `{"clientName": "...", "creationDate": "..."}`, not a plain string.
- **Email format**: `name@vpn` — derived from clientsTable name, used as xray stats identifier.
