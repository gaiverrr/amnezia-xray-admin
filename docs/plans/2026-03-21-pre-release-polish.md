# Pre-release polish: backups, CLI commands, UX

## Overview
Implement the design spec from `docs/superpowers/specs/2026-03-21-pre-release-polish-design.md`:
1. Auto-backup server.json + clientsTable before every mutation
2. CLI commands: --backup, --restore, --add-user, --delete-user, --rename-user
3. UX: TUI layout polish, --help examples, better error messages

## Context
- Design spec: `docs/superpowers/specs/2026-03-21-pre-release-polish-design.md`
- Key files: `src/xray/client.rs` (XrayApiClient mutations), `src/xray/config.rs` (ensure_api_enabled), `src/main.rs` (CLI dispatch), `src/config.rs` (Cli struct), `src/backend_trait.rs` (XrayBackend trait)
- Existing CLI pattern: flags in Cli struct → async handler in main.rs → connect_cli_backend → XrayApiClient
- Backup paths: `/opt/amnezia/xray/server.json.bak`, `/opt/amnezia/xray/clientsTable.bak` (inside container)
- Timestamp format: `YYYYMMDD-HHMMSS` from `$(date +%Y%m%d-%H%M%S)`

## Development Approach
- **Testing approach**: CLI-first — implement, verify via CLI, then check TUI
- Complete each task fully before moving to the next
- **CRITICAL: every task MUST include new/updated tests**
- **CRITICAL: all tests must pass before starting next task**
- **CRITICAL: update this plan file when scope changes**
- Run `cargo test` after each change
- Verify CLI commands against real VPS: `ssh vps-vpn`

## Testing Strategy
- **Unit tests**: backup_config command building, restore listing/parsing, rename logic, TTY detection, error message formatting
- **Integration via CLI**: `cargo run -- --backup`, `cargo run -- --add-user TestUser`, `cargo run -- --delete-user TestUser --yes`

## Progress Tracking
- Mark completed items with `[x]` immediately when done
- Add newly discovered tasks with ➕ prefix
- Document issues/blockers with ⚠️ prefix

## Implementation Steps

### Task 1: Add backup_config() to XrayApiClient
- [x] add `backup_config()` method to `XrayApiClient` in `src/xray/client.rs`: copies both server.json and clientsTable to `.bak` via `exec_in_container("cp ...")`
- [x] add `backup_config_timestamped()` for manual backups: uses `$(date +%Y%m%d-%H%M%S)` suffix
- [x] call `backup_config()` at the start of `add_user()` before any writes
- [x] call `backup_config()` at the start of `remove_user()` before any writes
- [x] call `backup_config()` in `ensure_api_enabled()` in `src/xray/config.rs` — only when `modified == true`, before `upload_and_restart()`
- [x] write tests for backup command string construction
- [x] run tests — must pass before next task

### Task 2: Add --backup CLI command
- [x] add `--backup` flag to `Cli` struct in `src/config.rs`
- [x] implement `cli_backup()` in `src/main.rs`: connect, call `backup_config_timestamped()`, print backup filenames
- [x] update test Cli struct instances with new field
- [x] verify: `cargo run -- --backup` (skipped - requires VPS connection)
- [x] run tests — must pass before next task

### Task 3: Add --restore CLI command
- [x] add `--restore` flag (optional value) to `Cli` struct
- [x] implement `cli_restore()` in `src/main.rs`:
  - list backups via `exec_in_container("ls -t /opt/amnezia/xray/server.json.*.bak")`
  - parse timestamps from filenames
  - validate both server.json.{ts}.bak and clientsTable.{ts}.bak exist
  - if no timestamp given: use latest; if timestamp given: use that one
  - copy .bak files back to originals via `exec_in_container("cp ...")`
  - restart container via `exec_on_host("docker restart <container>")`
- [x] handle error: "Incomplete backup: clientsTable.{ts}.bak not found"
- [x] update test Cli struct instances
- [x] write tests for timestamp parsing and backup listing logic
- [x] verify: `cargo run -- --restore` (skipped - requires VPS connection)
- [x] run tests — must pass before next task

### Task 4: Add --add-user CLI command
- [ ] add `--add-user <name>` arg to `Cli` struct
- [ ] implement `cli_add_user()` in `src/main.rs`: connect, call `client.add_user(name)`, generate vless URL, print formatted output:
  ```
  User added successfully.
  Name:  Friend
  UUID:  a1b2c3d4-...
  URL:   vless://...
  ```
- [ ] update test Cli struct instances
- [ ] verify: `cargo run -- --add-user TestUser`
- [ ] run tests — must pass before next task

### Task 5: Add --delete-user CLI command with confirmation
- [ ] add `--delete-user <name>` arg and `--yes` flag to `Cli` struct
- [ ] implement `cli_delete_user()` in `src/main.rs`:
  - if `--yes` flag: skip confirmation
  - if stdout is TTY (`std::io::stdin().is_terminal()`): prompt "Type user name to confirm deletion:"
  - if not TTY: fail with "Interactive confirmation required. Use --yes to skip."
  - call `client.remove_user(uuid)`
- [ ] update test Cli struct instances
- [ ] write tests for TTY detection logic and confirmation flow
- [ ] verify: `cargo run -- --delete-user TestUser --yes`
- [ ] run tests — must pass before next task

### Task 6: Add --rename-user CLI command
- [ ] add `--rename-user` arg (takes two values: old name, new name) to `Cli` struct
- [ ] implement `cli_rename_user()` in `src/main.rs`:
  - find user by old name
  - backup_config() before changes
  - call `exec_api_rmu` with old email (remove old stats counter)
  - update clientsTable: change clientName in userData
  - update server.json: change email field (old_name@vpn → new_name@vpn)
  - write both files, restart container via `upload_and_restart()`
  - print warning: "Note: rename resets traffic stats history for this user."
- [ ] add `rename_user()` method to `XrayApiClient`
- [ ] update test Cli struct instances
- [ ] write tests for rename logic (clientsTable update, email change)
- [ ] verify: `cargo run -- --rename-user TestUser NewName`
- [ ] run tests — must pass before next task

### Task 7: Improve --help with examples
- [ ] replace `long_about` in `Cli` struct with `after_help` containing usage examples
- [ ] include examples for: TUI launch, SSH alias, list-users, add-user, user-qr, deploy-bot, backup, restore
- [ ] verify: `cargo run -- --help`
- [ ] run tests — must pass before next task

### Task 8: Improve error messages
- [ ] wrap SSH connection errors in `src/ssh.rs` or `src/backend.rs` with actionable hints:
  - connection refused → "Cannot connect to VPS. Check: 1) SSH config alias is correct 2) VPS is reachable 3) SSH key is loaded"
  - auth failed → "SSH authentication failed. Check your SSH key or ssh-agent."
- [ ] wrap container errors: "Container 'X' not found. Run 'docker ps' on your VPS."
- [ ] wrap xray API errors: "Xray API not responding. Run '--check-server' to diagnose."
- [ ] wrap public key missing: "Public key file missing in container. Is Amnezia Xray properly installed?"
- [ ] write tests for error message wrapping
- [ ] run tests — must pass before next task

### Task 9: TUI layout polish
- [ ] review dashboard screen: column widths for names with brackets/spaces
- [ ] review user detail screen: alignment of labels and values
- [ ] review setup wizard: field spacing and alignment
- [ ] review add user dialog: input field width
- [ ] test with real data (connect to VPS, check all screens render correctly)
- [ ] run tests — must pass before next task

### Task 10: Verify acceptance criteria
- [ ] verify auto-backup works: add a user via CLI, check .bak files exist in container
- [ ] verify --backup creates timestamped backups
- [ ] verify --restore restores both files and restarts container
- [ ] verify --add-user prints formatted output with URL
- [ ] verify --delete-user requires confirmation (and --yes skips it)
- [ ] verify --rename-user changes name and warns about stats reset
- [ ] verify --help shows examples
- [ ] verify error messages are actionable
- [ ] run full test suite: `cargo test`
- [ ] run linter: `cargo clippy`

### Task 11: [Final] Update documentation
- [ ] update CLAUDE.md with new CLI commands
- [ ] update README.md CLI commands section
- [ ] update CHANGELOG.md with new features

## Technical Details

**Backup command construction:**
```rust
// Auto-backup (overwrites latest)
exec_in_container("cp /opt/amnezia/xray/server.json /opt/amnezia/xray/server.json.bak")
exec_in_container("cp /opt/amnezia/xray/clientsTable /opt/amnezia/xray/clientsTable.bak")

// Timestamped backup
exec_in_container("cp /opt/amnezia/xray/server.json /opt/amnezia/xray/server.json.$(date +%Y%m%d-%H%M%S).bak")
exec_in_container("cp /opt/amnezia/xray/clientsTable /opt/amnezia/xray/clientsTable.$(date +%Y%m%d-%H%M%S).bak")
```

**Restore flow:**
1. List: `exec_in_container("ls -t /opt/amnezia/xray/server.json.*.bak")`
2. Parse timestamps from filenames
3. Validate: check clientsTable.{ts}.bak exists too
4. Copy: `exec_in_container("cp server.json.{ts}.bak server.json && cp clientsTable.{ts}.bak clientsTable")`
5. Restart: `exec_on_host("docker restart <container>")`

**Rename flow:**
1. backup_config()
2. `build_rmu_cmd(old_email)` → exec_in_container (remove old stats)
3. Update clientsTable in memory (change clientName)
4. Update server.json in memory (change email field)
5. Write both files via exec_in_container
6. Restart container via upload_and_restart()

**TTY detection:**
```rust
use std::io::IsTerminal;
if !std::io::stdin().is_terminal() {
    return Err(AppError::Config("Interactive confirmation required. Use --yes to skip.".into()));
}
```

## Post-Completion
- Manual TUI testing on real VPS
- Test backup/restore cycle end-to-end
- Test rename with active VPN connection (verify UUID-based connections survive)
