# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # dev build
cargo build --release          # release build
cargo test                     # all 267 tests
cargo test xray::client        # run tests in one module
cargo test test_build_rmu      # run a single test by name
cargo clippy                   # lint (expect dead_code warnings - backend not fully wired)
cargo fmt --check              # format check
cargo run                      # launch TUI
cargo run -- --ssh-host vps-vpn --container amnezia-xray  # with CLI args
```

## Architecture

**Async TUI pattern**: Synchronous ratatui event loop + tokio runtime for background SSH/Xray operations. Communication via `mpsc` channel (`BackendMsg` enum).

```
main.rs → App (state machine + event loop)
             ├── backend.rs (spawns async tasks on tokio, sends results via mpsc)
             ├── ui/ (screen rendering: setup, dashboard, user_detail, add_user, qr)
             └── xray/ (client.rs → ssh.rs → remote docker exec → xray API)
```

**Key flow**: TUI calls `backend::spawn_*()` → tokio task runs SSH commands → sends `BackendMsg` back → `App::process_backend_messages()` updates state → next `draw()` renders.

**Guard flags** (`pending_refresh`, `pending_add_name`, etc.) prevent duplicate async operations. `refresh_after_mutation` handles stale fetch results after add/delete.

## Module Responsibilities

- **config.rs**: TOML config at `~/.config/amnezia-xray-admin/config.toml`, clap CLI args, merge logic
- **ssh.rs**: Pure-Rust SSH via russh. Parses `~/.ssh/config`, TOFU known_hosts verification, `exec_command()` / `exec_in_container()`
- **xray/types.rs**: Data types (`XrayUser`, `ServerConfig`, `ClientsTable`). `ServerConfig` wraps `serde_json::Value` to preserve unknown fields
- **xray/config.rs**: `ensure_api_enabled()` — one-time server.json transformation (adds stats/policy/api sections, emails, level:0)
- **xray/client.rs**: `XrayApiClient` — list/add/remove users, stats, online status. Commands run via `docker exec <container> xray api ...`
- **backend.rs**: Async task spawners, `BackendMsg` enum, connection helpers
- **app.rs**: 5-screen state machine (Setup→Dashboard→UserDetail/AddUser/QrView), event loop with 250ms poll + 5s auto-refresh

## Important Design Details

- **No bind mounts**: Amnezia stores config files *inside* the container at `/opt/amnezia/xray/`. All file reads/writes must use `exec_in_container()`, not `exec_command()`.
- **Shell safety**: User names can contain brackets/spaces (e.g., "Admin [macOS Tahoe]"). Use `shell_quote()` for xray API commands, base64 for JSON payloads.
- **Stats require `level: 0`**: Xray only tracks per-user traffic when clients have `"level": 0` matching the policy section.
- **clientsTable format**: `userData` is an object `{"clientName": "...", "creationDate": "..."}`, not a plain string.
- **Email format**: `name@vpn` — derived from clientsTable name, used as xray stats identifier.
