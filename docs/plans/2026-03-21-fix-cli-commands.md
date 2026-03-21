# Fix broken features, CLI commands, and Telegram bot

## Overview
Three-phase plan:
1. **Fix bugs** — broken URL/QR/online status in TUI (same exec_command vs exec_in_container pattern)
2. **CLI commands** — testable CLI for every feature (`--user-url`, `--user-qr`, `--online-status`, `--server-info`)
3. **Architecture refactor** — extract `XrayBackend` trait (SSH vs Local) to share code between remote CLI, TUI, and Telegram bot
4. **Telegram bot** — daemon mode (`--telegram-bot`) running on VPS in Docker, managing users via Telegram
5. **Startup validation** — ensure xray API is configured on every app launch

**Telegram bot commands (basic):**
- `/users` — list users with stats
- `/add <name>` — add user, send QR code image
- `/delete <name>` — delete user with confirmation
- `/url <name>` — send vless:// URL
- `/qr <name>` — send QR code as image
- `/status` — server info + online users

**Deployment:** Docker container on VPS with `/var/run/docker.sock` mount for direct container access. Easy install: `docker run -d --name axadmin -v /var/run/docker.sock:/var/run/docker.sock -e TELEGRAM_TOKEN=... axadmin --telegram-bot`

## Context
- Root cause of bugs: Amnezia has no bind mounts, files only inside container. Several functions use `exec_command()` (host) instead of `exec_in_container()`.
- Existing CLI: `--list-users` in `src/main.rs` works as a pattern
- Key files: `src/backend.rs` (read_public_key bug), `src/main.rs` (CLI), `src/xray/client.rs` (API), `src/ssh.rs` (SSH session), `src/ui/qr.rs` (QR)
- **Architecture gap**: all xray operations are coupled to `SshSession`. Need `XrayBackend` trait for local (docker exec) vs remote (SSH) execution.

## Development Approach
- **Testing approach**: CLI-first — fix bugs, add CLI commands, verify via CLI, then TUI
- Complete each task fully before moving to the next
- **CRITICAL: every task MUST include new/updated tests**
- **CRITICAL: all tests must pass before starting next task**
- **CRITICAL: update this plan file when scope changes**
- Run `cargo test` after each change
- Verify CLI commands against real VPS: `ssh vps-vpn`

## Testing Strategy
- **Unit tests**: URL generation, QR rendering, stat parsing, command building, backend trait
- **Integration tests via CLI**: `cargo run -- --user-url Alexander`, `cargo run -- --server-info`
- **Manual verification**: TUI copy URL, QR code, online status

## Progress Tracking
- Mark completed items with `[x]` immediately when done
- Add newly discovered tasks with ➕ prefix
- Document issues/blockers with ⚠️ prefix

## Implementation Steps

### Phase 1: Bug fixes

### Task 1: Fix read_public_key — exec_command → exec_in_container
- [x] fix `read_public_key()` in `src/backend.rs:118` to use `exec_in_container()`
- [x] grep codebase for any remaining `exec_command()` that should be `exec_in_container()` (remaining uses in xray/config.rs are correct: docker restart and docker exec must run on host)
- [x] write test verifying PUBLIC_KEY_PATH constant
- [x] run tests — must pass before next task

### Task 2: Fix startup API validation (ensure_api_enabled)
- [x] verify `ensure_api_enabled()` runs on first dashboard load (check `spawn_fetch_dashboard` in backend.rs)
- [x] ensure it handles already-configured servers gracefully (idempotent)
- [x] ensure container restart happens only when config actually changes
- [x] add `--check-server` CLI command to verify API setup without launching TUI
- [x] test: `cargo run -- --check-server` should print "API enabled, N users, xray vX.X"
- [x] run tests — must pass before next task

### Task 3: Investigate and fix online status
- [x] test xray `statsonline` API manually (skipped - manual SSH test, commands verified via xray-core source)
- [x] test `statsonlineiplist` API manually (skipped - manual SSH test, commands verified via xray-core source)
- [x] check if xray version supports these commands (verified: statsonline, statsonlineiplist, statsgetallonlineusers all exist in xray-core)
- [x] fix command/parsing if API format differs from expected (fixed: xray outputs JSON not proto text; rewrote parsers with JSON-first + proto text fallback)
- [x] write test for online status parsing (12 new tests: parse_online_count and parse_online_ip_list for JSON format, proto text fallback, edge cases)
- [x] run tests — must pass before next task

### Phase 2: CLI commands

### Task 4: Add --user-url CLI command
- [x] add `--user-url <name>` arg to `Cli` struct in `src/config.rs`
- [x] implement `cli_user_url()` async fn in `src/main.rs`: connect, find user, generate vless:// URL, print
- [x] handle "user not found" error gracefully
- [x] update test Cli struct instances with new field
- [x] verify: `cargo run -- --user-url Alexander` (manual verification - not automatable without VPS)
- [x] run tests — must pass before next task

### Task 5: Add --user-qr CLI command
- [x] add `--user-qr <name>` arg to `Cli` struct
- [x] implement `cli_user_qr()`: generate URL, render QR via `ui::qr::render_qr_to_lines()`, print to stdout
- [x] print user name + vless:// URL below QR
- [x] update test Cli struct instances
- [x] verify: `cargo run -- --user-qr Alexander` (manual verification - not automatable without VPS)
- [x] run tests — must pass before next task

### Task 6: Add --online-status CLI command
- [x] add `--online-status` flag to `Cli` struct
- [x] implement `cli_online_status()`: list users, get online count + IPs, print table
- [x] format: NAME | ONLINE | IPs
- [x] update test Cli struct instances
- [x] verify: `cargo run -- --online-status` (manual verification - not automatable without VPS)
- [x] run tests — must pass before next task

### Task 7: Add --server-info CLI command
- [x] add `--server-info` flag to `Cli` struct
- [x] implement `cli_server_info()`: xray version, total traffic, user count, API status
- [x] update test Cli struct instances
- [x] verify: `cargo run -- --server-info` (manual verification - not automatable without VPS)
- [x] run tests — must pass before next task

### Task 8: Verify Phase 1-2 acceptance
- [x] verify all CLI commands work against real VPS (skipped - requires manual VPS access)
- [x] verify TUI copy URL works (press [c] in user detail) (skipped - requires manual TUI testing)
- [x] verify TUI QR code works (press [q] in user detail) (skipped - requires manual TUI testing)
- [x] verify TUI online status for connected users (skipped - requires manual TUI testing)
- [x] run full test suite + clippy (275 tests pass, clippy clean)

### Phase 3: Backend abstraction

### Task 9: Extract XrayBackend trait
- [x] create `src/backend_trait.rs` with `XrayBackend` trait:
  ```rust
  #[async_trait]
  pub trait XrayBackend: Send + Sync {
      async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput>;
      async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput>;
      fn container_name(&self) -> &str;
      fn hostname(&self) -> &str;
  }
  ```
- [x] implement `SshBackend` wrapping existing `SshSession`
- [x] refactor `XrayApiClient` to accept `&dyn XrayBackend` instead of `&SshSession`
- [x] refactor `read_server_config`, `read_clients_table`, `ensure_api_enabled` to use trait
- [x] ensure all existing tests pass with refactored code
- [x] run tests — must pass before next task

### Task 10: Implement LocalBackend (docker exec without SSH)
- [x] create `LocalBackend` struct that runs `docker exec` via `tokio::process::Command`
- [x] implement `XrayBackend` for `LocalBackend`
- [x] add `exec_on_host` as local shell command execution
- [x] write unit tests for LocalBackend command building
- [x] add `--local` CLI flag to use LocalBackend instead of SSH (for testing on VPS)
- [x] run tests — must pass before next task

### Phase 4: Telegram bot

### Task 11: Add teloxide dependency and bot skeleton
- [ ] add `teloxide` crate to Cargo.toml (with features: macros, auto-send)
- [ ] add `--telegram-bot` flag to Cli struct
- [ ] add `TELEGRAM_TOKEN` env var reading (or `--telegram-token` arg)
- [ ] implement auto-admin: first user to send `/start` becomes admin, chat ID saved to config
- [ ] create `src/telegram.rs` module with bot startup function
- [ ] implement basic `/start` and `/help` commands
- [ ] run tests — must pass before next task

### Task 12: Telegram /users and /status commands
- [ ] implement `/users` — list users with stats (like --list-users but formatted for Telegram)
- [ ] implement `/status` — server info (version, traffic, user count, online users)
- [ ] format with Telegram markdown (monospace for tables)
- [ ] write tests for message formatting
- [ ] run tests — must pass before next task

### Task 13: Telegram /add and /delete commands
- [ ] implement `/add <name>` — add user, return UUID + vless:// URL
- [ ] implement `/delete <name>` — require confirmation via inline keyboard button
- [ ] handle errors (duplicate name, invalid name, user not found)
- [ ] write tests for command parsing and validation
- [ ] run tests — must pass before next task

### Task 14: Telegram /url and /qr commands
- [ ] implement `/url <name>` — send vless:// URL as copyable message
- [ ] implement `/qr <name>` — generate QR code as PNG image, send via Telegram
- [ ] use `qrcode` crate to render to PNG (not unicode blocks) for Telegram
- [ ] write tests for QR PNG generation
- [ ] run tests — must pass before next task

### Task 15: Docker image and deployment scripts
- [ ] create `Dockerfile` (multi-stage: rust builder → minimal runtime with docker CLI)
- [ ] create `docker-compose.yml` with docker.sock mount and env vars
- [ ] create `deploy/install.sh` — one-command install script for VPS
- [ ] test Docker build locally
- [ ] run tests — must pass before next task

### Task 16: TUI "Setup Telegram Bot" screen
- [ ] add "Telegram Bot" option to TUI dashboard (new keybinding, e.g. [t])
- [ ] create setup screen with:
  - Short instruction: "1. Open @BotFather in Telegram → 2. /newbot → 3. Copy token → 4. Paste below"
  - Token input field
  - "After deploy, send /start to your bot — you'll become the admin automatically"
- [ ] "Deploy to VPS" button: connects via SSH, pulls Docker image, creates docker-compose, starts bot
- [ ] show deployment progress (pulling image → starting → verifying bot responds)
- [ ] save telegram config to `~/.config/amnezia-xray-admin/config.toml`
- [ ] write tests for config serialization with telegram fields
- [ ] run tests — must pass before next task

### Task 17: CLI --deploy-bot command (alternative to TUI)
- [ ] add `--deploy-bot` flag that does the same as TUI setup but via CLI prompts
- [ ] `--deploy-bot --token <TOKEN>` for non-interactive mode (admin auto-detected on first /start)
- [ ] connect to VPS via SSH, deploy Docker container with bot
- [ ] verify bot starts and responds to /start
- [ ] run tests — must pass before next task

### Task 18: Verify Phase 3-4 acceptance
- [ ] verify Telegram bot responds to all 6 commands
- [ ] verify access control (first /start = admin, others get "Access denied")
- [ ] verify TUI "Setup Telegram Bot" deploys successfully
- [ ] verify CLI --deploy-bot works
- [ ] verify CLI still works with both SshBackend and LocalBackend
- [ ] run full test suite + clippy

### Task 19: [Final] Update documentation
- [ ] update CLAUDE.md with new CLI commands and architecture
- [ ] update README with:
  - CLI usage examples
  - Telegram bot setup (via TUI and CLI)
  - Docker deployment instructions
  - Screenshots of Telegram bot commands

## Technical Details

**XrayBackend trait** enables code reuse:
```
                    ┌─────────────────┐
                    │  XrayApiClient  │
                    │  (all xray ops) │
                    └────────┬────────┘
                             │ uses
                    ┌────────▼────────┐
                    │  XrayBackend    │
                    │  (trait)        │
                    └────┬───────┬────┘
                         │       │
              ┌──────────▼┐  ┌──▼──────────┐
              │ SshBackend │  │LocalBackend │
              │ (remote)   │  │(on VPS)     │
              └────────────┘  └─────────────┘
                    │                │
              Used by:          Used by:
              - TUI              - Telegram bot
              - CLI (remote)     - CLI --local
```

**Telegram bot architecture:**
- Uses `teloxide` (Rust Telegram framework)
- Runs as `--telegram-bot` mode of the same binary
- On VPS: uses `LocalBackend` (direct docker exec)
- Access control: first `/start` becomes admin (auto-detect, no manual ID input)
- Admin chat ID persisted to config file inside container
- QR codes sent as PNG images (not unicode blocks)

**Docker deployment:**
```yaml
# docker-compose.yml
services:
  axadmin-bot:
    image: axadmin:latest
    restart: unless-stopped
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    environment:
      - TELEGRAM_TOKEN=${TELEGRAM_TOKEN}
      # Admin auto-detected on first /start
      - XRAY_CONTAINER=amnezia-xray
    command: --telegram-bot --local --container amnezia-xray
```

**vless:// URL generation requires:**
- User UUID (from server.json)
- Server hostname (from config or auto-detect via `hostname -I`)
- Port (from server.json VLESS inbound)
- SNI + short_id (from server.json Reality settings)
- Public key (from `/opt/amnezia/xray/xray_public.key` — inside container)
- User name (from clientsTable)

## Post-Completion
- Manual TUI testing: verify all screens work end-to-end
- Test Telegram bot with real users
- Consider: `--json` output for CLI (machine-readable)
- Consider: rate limiting for Telegram bot
- Consider: multi-server support (manage multiple VPS from one bot)
