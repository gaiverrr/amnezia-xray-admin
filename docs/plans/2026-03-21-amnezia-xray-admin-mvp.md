# amnezia-xray-admin MVP

## Overview
A hacker-aesthetic TUI dashboard for managing Amnezia VPN's Xray (VLESS + XTLS-Reality) server.
Connects to VPS via pure-Rust SSH, talks to Xray gRPC API for live user management and traffic stats.
Open-source friendly: first-run setup wizard, config file, SSH config alias support.

**Problem:** Amnezia client app manages users by SSH-ing in, editing JSON, and restarting the container.
No dashboard, no stats, no live view. This tool fills that gap.

**Key features (MVP):**
- List users with online/offline status and traffic stats
- Add/remove users (live via Xray gRPC API, no restart)
- Generate `vless://` URL + terminal QR code for easy sharing
- Fancy hacker/cyberpunk TUI aesthetic

## Context
- **Server:** VPS accessible via `ssh vps-vpn`, running `amnezia-xray` Docker container
- **Protocol:** VLESS + XTLS-Reality on port 443, masquerading as `www.googletagmanager.com`
- **Xray version:** 25.8.3 with gRPC API support (`HandlerService`, `StatsService`)
- **Current users:** 8 clients (1 initial UUID + 7 named in `clientsTable`)
- **Server config:** `/opt/amnezia/xray/server.json` inside container
- **Client registry:** `/opt/amnezia/xray/clientsTable` (JSON array with clientId + userData)
- **Keys on server:** `xray_uuid.key`, `xray_public.key`, `xray_private.key`, `xray_short_id.key`
- **API not yet enabled** — needs `api`, `stats`, `policy` sections added to server.json
- **Xray API capabilities:** `adu`/`rmu` (add/remove users live), `stats`/`statsquery`, `statsonline`, `statsonlineiplist`

## Tech Stack
- **Rust 1.94** (edition 2021)
- **ratatui** — TUI framework
- **crossterm** — terminal backend
- **russh** — pure Rust SSH2 client (connection, exec, port-forwarding)
- **tokio** — async runtime
- **serde / serde_json** — JSON parsing (server.json, clientsTable)
- **uuid** — UUID generation for new clients
- **clap** — CLI argument parsing
- **toml** — config file parsing
- **dirs** — XDG config directory resolution
- **qrcode + unicode-width** — terminal QR code rendering
- **base64** — vless:// URL encoding

## Development Approach
- **Testing approach**: Regular (code first, then tests)
- Complete each task fully before moving to the next
- Make small, focused changes
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task
- **CRITICAL: all tests must pass before starting next task**
- **CRITICAL: update this plan file when scope changes during implementation**
- Open-source friendly: good error messages, config file docs, --help output

## Architecture

```
src/
├── main.rs           # Entry point, CLI args, app bootstrap
├── app.rs            # App state machine, event loop
├── config.rs         # Config file (~/.config/amnezia-xray-admin/config.toml)
├── ssh.rs            # SSH connection via russh (exec, port-forward)
├── xray/
│   ├── mod.rs        # Re-exports
│   ├── client.rs     # Xray gRPC API client (stats, user mgmt)
│   ├── config.rs     # server.json / clientsTable parsing & mutation
│   └── types.rs      # Data types (User, Stats, ServerConfig)
├── ui/
│   ├── mod.rs        # Re-exports
│   ├── theme.rs      # Hacker/cyberpunk color palette & styles
│   ├── dashboard.rs  # Main dashboard view (user list + stats)
│   ├── setup.rs      # First-run setup wizard
│   ├── user_detail.rs # User detail panel (IPs, traffic, actions)
│   ├── add_user.rs   # Add user dialog
│   └── qr.rs         # QR code display widget
└── error.rs          # Error types
```

## Progress Tracking
- Mark completed items with `[x]` immediately when done
- Add newly discovered tasks with + prefix
- Document issues/blockers with ! prefix
- Update plan if implementation deviates from original scope

## Implementation Steps

### Task 1: Project scaffolding and dependencies
- [x] fix Cargo.toml edition (2024 -> 2021) and add all dependencies
- [x] create module structure (empty files with mod declarations)
- [x] define error types in `src/error.rs` (AppError enum with SSH, Xray, Config, IO variants)
- [x] write tests for error type Display/From impls
- [x] `cargo build` must succeed

### Task 2: Config system
- [x] define config struct in `src/config.rs` (host, port, user, key_path, ssh_config_host, container_name)
- [x] implement config loading from `~/.config/amnezia-xray-admin/config.toml`
- [x] implement config saving with `0600` permissions
- [x] implement CLI args with clap (--host, --port, --user, --key, --ssh-host, --container)
- [x] CLI args override config file values; --ssh-host uses SSH config alias (e.g. `vps-vpn`)
- [x] write tests for config loading, merging, defaults
- [x] run tests — must pass before next task

### Task 3: SSH connection layer
- [x] implement SSH connection in `src/ssh.rs` using russh
- [x] support direct host:port connection with key auth
- [x] support ssh-agent authentication
- [x] parse `~/.ssh/config` to resolve Host aliases (hostname, port, user, identity file)
- [x] implement `exec_command()` — run command on remote, return stdout/stderr
- [x] implement `exec_in_container()` — wrapper for `docker exec <container> <cmd>`
- [x] write tests for SSH config parsing (unit tests with mock config content)
- [x] run tests — must pass before next task

### Task 4: Xray data types and server config parsing
- [x] define types in `src/xray/types.rs`: `XrayUser`, `ClientEntry`, `ServerConfig`, `TrafficStats`
- [x] implement `ServerConfig` parsing from server.json in `src/xray/config.rs`
- [x] implement `ClientsTable` parsing from clientsTable JSON
- [x] implement merging: cross-reference server.json clients with clientsTable names
- [x] implement server.json mutation: add/remove client from clients array
- [x] implement clientsTable mutation: add/remove entry
- [x] write tests with fixture JSON (real format from server)
- [x] run tests — must pass before next task

### Task 5: Enable Xray API on server (one-time setup)
- [x] implement `ensure_api_enabled()` in `src/xray/config.rs`
- [x] detect if server.json already has `api` section
- [x] if missing: add `api`, `stats`, `policy`, `routing` rule, and `dokodemo-door` inbound on 127.0.0.1:8080
- [x] add `email` field to each existing client (derive from clientsTable name or UUID)
- [x] add `tag: "vless-in"` to the main inbound
- [x] upload modified config and restart container via SSH
- [x] write tests for config transformation (input JSON -> expected output JSON)
- [x] run tests — must pass before next task

### Task 6: Xray API client (stats and user management)
- [x] implement gRPC communication via SSH-tunneled commands in `src/xray/client.rs`
- [x] `list_users()` — read server.json + clientsTable, return merged user list
- [x] `add_user(name)` — generate UUID, call `xray api adu`, update server.json + clientsTable
- [x] `remove_user(uuid)` — call `xray api rmu`, update server.json + clientsTable
- [x] `get_user_stats(email)` — call `xray api stats` for up/down traffic
- [x] `get_online_count(email)` — call `xray api statsonline`
- [x] `get_online_ips(email)` — call `xray api statsonlineiplist`
- [x] `get_server_info()` — xray version, uptime, total traffic
- [x] write tests for command construction and response parsing
- [x] run tests — must pass before next task

### Task 7: vless:// URL and QR code generation
- [x] implement `generate_vless_url()` in `src/xray/client.rs`
- [x] format: `vless://<uuid>@<host>:443?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=<site>&fp=chrome&pbk=<pubkey>&sid=<shortid>#<name>`
- [x] implement QR code rendering as unicode block characters for terminal display
- [x] write tests for URL generation (known inputs -> expected URL)
- [x] run tests — must pass before next task

### Task 8: TUI theme and base layout
- [x] define hacker/cyberpunk theme in `src/ui/theme.rs`
- [x] color palette: matrix green (#00ff41), cyan (#00d4ff), dark backgrounds, red for alerts
- [x] ASCII art header/logo for the app
- [x] implement base app frame with ratatui: header bar, main area, status bar, keybinding hints
- [x] implement app state machine in `src/app.rs` (screens: Setup, Dashboard, UserDetail, AddUser, QrView)
- [x] implement event loop: key input handling, periodic refresh
- [x] `cargo build` must succeed, app launches and shows empty dashboard frame

### Task 9: First-run setup wizard TUI
- [x] implement setup screen in `src/ui/setup.rs`
- [x] input fields: host, port (default 22), user (default root), SSH key path, SSH config alias, container name (default amnezia-xray)
- [x] tab/shift-tab navigation between fields
- [x] [Test Connection] button — attempts SSH + reads xray version
- [x] [Save & Start] — saves config and transitions to dashboard
- [x] show connection test result (success with version or error message)
- [x] if config already exists, skip setup and go to dashboard
- [x] manual testing: run app, complete setup flow [x] (skipped - not automatable)

### Task 10: Dashboard view — user list with stats
- [x] implement dashboard in `src/ui/dashboard.rs`
- [x] server info header: hostname, xray version, total upload/download
- [x] user table: name, UUID (truncated), upload, download, online status (green dot/red dot), online count
- [x] keyboard navigation: j/k or arrow keys to select user, Enter for detail
- [x] `[a]` keybinding hint for add user, `[d]` for delete, `[r]` for refresh, `[q]` for quit
- [x] auto-refresh stats on configurable interval (default 5s)
- [x] loading spinner while fetching data
- [x] manual testing: connect to real server, see user list with live data (skipped - not automatable)

### Task 11: Add user dialog
- [x] implement add user dialog in `src/ui/add_user.rs`
- [x] modal overlay on dashboard
- [x] input field for user name
- [x] on confirm: call `add_user()`, show success + vless:// URL, offer QR view
- [x] on cancel: dismiss modal
- [x] error handling: show error message if add fails
- [x] manual testing: add a test user, verify appears in list (skipped - not automatable)

### Task 12: User detail panel and delete confirmation
- [x] implement user detail in `src/ui/user_detail.rs`
- [x] show: full UUID, name, creation date, upload/download, online IPs with timestamps
- [x] `[q]` to show QR code, `[d]` to delete with confirmation
- [x] delete confirmation: "Are you sure? Type user name to confirm" (prevents accidental deletion)
- [x] `[c]` to copy vless:// URL to clipboard (if terminal supports OSC 52)
- [x] manual testing: view user details, delete a test user (skipped - not automatable)

### Task 13: QR code view
- [x] implement QR display in `src/ui/qr.rs`
- [x] render QR code centered in terminal using unicode half-blocks
- [x] show vless:// URL below QR code
- [x] show user name as title
- [x] `Esc` or `q` to go back
- [x] manual testing: generate QR, scan with phone Amnezia app (skipped - not automatable)

### Task 14: Verify acceptance criteria
- [ ] verify all MVP features work end-to-end on real server
- [ ] verify first-run setup wizard works for new user
- [ ] verify SSH config alias (e.g. `vps-vpn`) works
- [ ] verify add/remove user works without container restart
- [ ] verify traffic stats update in real-time
- [ ] verify QR code scannable by Amnezia mobile app
- [ ] run full test suite (unit tests)
- [ ] run `cargo clippy` — all warnings must be fixed
- [ ] run `cargo fmt --check` — code must be formatted

### Task 15: [Final] Documentation and open-source prep
- [ ] write README.md with: description, screenshots placeholder, installation, usage, configuration, building from source
- [ ] add LICENSE file (MIT or Apache-2.0)
- [ ] add `--help` output that's genuinely helpful
- [ ] ensure no secrets/hardcoded values in codebase

## Technical Details

### Server Config Transformation (Task 5)
Current server.json needs these additions to enable the API:
```json
{
  "api": { "tag": "api", "services": ["HandlerService", "StatsService"] },
  "stats": {},
  "policy": {
    "levels": { "0": { "statsUserUplink": true, "statsUserDownlink": true } },
    "system": { "statsInboundUplink": true, "statsInboundDownlink": true }
  },
  "routing": { "rules": [{ "inboundTag": ["api"], "outboundTag": "api", "type": "field" }] },
  "inbounds": [
    { "tag": "api", "port": 8080, "listen": "127.0.0.1", "protocol": "dokodemo-door", "settings": { "address": "127.0.0.1" } },
    { "tag": "vless-in", "port": 443, ... }  // existing inbound with tag + email fields on clients
  ]
}
```

Each client needs `"email": "<name>@vpn"` for stats tracking.

### vless:// URL Format
```
vless://<uuid>@<server_ip>:<port>?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=<masking_site>&fp=chrome&pbk=<public_key>&sid=<short_id>#<url_encoded_name>
```

### Xray API Commands (executed via `docker exec amnezia-xray xray api ...`)
- Add user: `xray api adu -s 127.0.0.1:8080 user.json` where user.json = `{"inboundTag":"vless-in","user":{"email":"name@vpn","level":0,"account":{"id":"<uuid>","flow":"xtls-rprx-vision"}}}`
- Remove user: `xray api rmu -s 127.0.0.1:8080 -email name@vpn`
- Stats: `xray api stats -s 127.0.0.1:8080 -name "user>>>name@vpn>>>traffic>>>downlink"`
- Online: `xray api statsonline -s 127.0.0.1:8080 -email name@vpn`

### TUI Color Palette (Hacker/Cyberpunk)
- Background: `#0a0a0a` (near-black)
- Primary text: `#00ff41` (matrix green)
- Secondary text: `#00d4ff` (cyan)
- Accent: `#ff00ff` (magenta)
- Alert/danger: `#ff0040` (neon red)
- Muted: `#444444` (dark gray)
- Success: `#00ff41` (green)
- Borders: `#1a3a1a` (dark green)

## Post-Completion

**Manual verification:**
- Test on Linux terminal (not just macOS)
- Test with different terminal emulators (iTerm2, Alacritty, kitty)
- Verify QR scanning works with Amnezia iOS and Android apps
- Test with slow SSH connections

**Future features (v2):**
- Speed limiting per user (tc/iptables via SSH)
- Connection logs viewer
- Bandwidth graphs over time (sparklines in TUI)
- Multiple server support
- Config backup/restore
- Amnezia WireGuard protocol support
