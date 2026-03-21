# Pre-release polish for v0.1.0

## Overview
Three areas of improvement before public release: safety (backups + confirmation), missing CLI commands (add/delete/rename), and UX polish (TUI layout, help examples, error messages).

## Safety: Auto-backup before mutations

Every operation that modifies server.json or clientsTable creates a backup first:
- `server.json.bak` and `clientsTable.bak` inside the container (overwritten each time — only latest backup kept)
- Backup happens automatically in `XrayApiClient::add_user()` and `remove_user()` before any writes
- In `ensure_api_enabled()`: backup only when `modified == true`, before calling `upload_and_restart()`
- Implementation: add `backup_config()` method to `XrayApiClient` that copies BOTH files via `exec_in_container("cp ...")`

### CLI backup/restore commands
- `--backup` — manual backup with timestamp. Both files backed up together:
  - `server.json.20260321-170000.bak` and `clientsTable.20260321-170000.bak`
  - Filename format from `$(date +%Y%m%d-%H%M%S)` — canonical, no dashes between date parts
- `--restore` — list available backup timestamps, restore BOTH server.json and clientsTable from the same timestamp, then restart container via `exec_on_host("docker restart <container>")`
- `--restore <timestamp>` — restore from specific timestamp (e.g. `--restore 20260321-170000`)
- Restore always restores both files atomically. If a clientsTable backup is missing for a timestamp, abort with error: "Incomplete backup: clientsTable.{ts}.bak not found"
- Container restart after restore uses `upload_and_restart()` pattern (exec_on_host for docker restart)

### Delete confirmation
- CLI `--delete-user <name>` requires either `--yes` flag (skip confirmation) or interactive prompt: "Type user name to confirm deletion:"
- When stdout is not a TTY (piped), fail with error: "Interactive confirmation required. Use --yes to skip." (detect via `std::io::IsTerminal`)
- Telegram `/delete <name>` already has inline keyboard confirmation — no change needed
- TUI already has type-name-to-confirm — no change needed

## CLI commands: add/delete/rename

### --add-user \<name\>
- Creates user with generated UUID
- Auto-backup before mutation
- Output format:
  ```
  User added successfully.
  Name:  Friend
  UUID:  a1b2c3d4-5678-90ab-cdef-1234567890ab
  URL:   vless://a1b2c3d4-...
  ```
- Works with `--local` flag

### --delete-user \<name\>
- Finds user by name, removes from server.json + clientsTable + xray runtime
- Auto-backup before mutation
- Requires `--yes` or interactive confirmation (TTY-aware, see above)
- Works with `--local` flag

### --rename-user \<old\> \<new\>
- Changes `clientName` in clientsTable `userData` object
- Changes `email` field in server.json (old_name@vpn → new_name@vpn)
- Does NOT change UUID — existing connections continue working
- Before disk write: calls `exec_api_rmu` with old email to remove runtime stats counter (matches remove_user pattern)
- Restarts container via `upload_and_restart()` for new email to take effect
- **Note:** rename resets traffic stats history for that user (xray counters are per-email, restart clears them). This is documented in --help and printed as a warning.
- Auto-backup before mutation
- Works with `--local` flag

## UX polish

### TUI layout
- Review all screens for spacing/alignment issues
- Ensure long names with brackets/spaces display correctly without truncation
- Adjust column widths in dashboard table for real-world data

### --help with examples
Replace existing `long_about` in Cli struct with `after_help` containing usage examples (avoids duplication):
```
Examples:
  amnezia-xray-admin                           # Launch TUI
  amnezia-xray-admin --ssh-host vps-vpn        # Connect via SSH alias
  amnezia-xray-admin --list-users              # List all users
  amnezia-xray-admin --add-user "Friend"       # Add user, get URL
  amnezia-xray-admin --user-qr "Friend"        # Show QR code
  amnezia-xray-admin --deploy-bot --telegram-token <TOKEN>
```

### Error messages
Improve common failure messages:
- SSH connection failed → "Cannot connect to VPS. Check: 1) SSH config alias is correct 2) VPS is reachable 3) SSH key is loaded (ssh-add)"
- Container not found → "Container 'X' not found. Run 'docker ps' on your VPS to find the correct container name."
- Xray API unreachable → "Xray API not responding. Run '--check-server' to diagnose."
- Public key not found → "Public key file missing in container. Is Amnezia Xray properly installed?"

## Technical notes
- `backup_config()` copies BOTH server.json and clientsTable via `exec_in_container("cp ...")`
- Timestamped backups use `exec_in_container("cp ... server.json.$(date +%Y%m%d-%H%M%S).bak")` — format: `YYYYMMDD-HHMMSS`
- `--restore` lists backups via `exec_in_container("ls -t /opt/amnezia/xray/server.json.*.bak")`, extracts timestamps, validates both files exist
- Restore copies files inside container via `exec_in_container`, then restarts via `exec_on_host("docker restart <container>")`
- `--rename-user`: calls rmu with old email → writes disk → restarts container
- All new CLI commands follow existing pattern: add flag to `Cli` struct, implement async handler in `main.rs`, update test Cli instances
- TTY detection for interactive confirmation: use `std::io::stdin().is_terminal()` (Rust 1.70+ std)

## Out of scope
- Multiple backup retention policy (keep N backups) — v0.2.0
- Undo last operation — v0.2.0
- Export/import config to local machine — v0.2.0
