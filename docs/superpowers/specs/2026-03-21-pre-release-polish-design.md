# Pre-release polish for v0.1.0

## Overview
Three areas of improvement before public release: safety (backups + confirmation), missing CLI commands (add/delete/rename), and UX polish (TUI layout, help examples, error messages).

## Safety: Auto-backup before mutations

Every operation that modifies server.json or clientsTable creates a backup first:
- `server.json.bak` and `clientsTable.bak` inside the container (overwritten each time — only latest backup kept)
- Backup happens automatically in `XrayApiClient::add_user()`, `remove_user()`, and `ensure_api_enabled()` before any writes
- Implementation: add `backup_config()` method to `XrayApiClient` that copies files via `exec_in_container("cp ...")`

### CLI backup/restore commands
- `--backup` — manual backup with timestamp: `server.json.2026-03-21-170000.bak`. Stored inside container.
- `--restore` — list available backups, restore from latest `.bak`, restart container
- `--restore <timestamp>` — restore from specific backup

### Delete confirmation
- CLI `--delete-user <name>` requires either `--yes` flag (skip confirmation) or interactive prompt: "Type user name to confirm deletion:"
- Telegram `/delete <name>` already has inline keyboard confirmation — no change needed
- TUI already has type-name-to-confirm — no change needed

## CLI commands: add/delete/rename

### --add-user \<name\>
- Creates user with generated UUID
- Auto-backup before mutation
- Prints: name, UUID, vless:// URL
- Works with `--local` flag

### --delete-user \<name\>
- Finds user by name, removes from server.json + clientsTable + xray runtime
- Auto-backup before mutation
- Requires `--yes` or interactive confirmation
- Works with `--local` flag

### --rename-user \<old\> \<new\>
- Changes `clientName` in clientsTable `userData` object
- Changes `email` field in server.json (old_name@vpn → new_name@vpn)
- Does NOT change UUID — existing connections continue working
- Requires container restart for email change to take effect in xray stats
- Auto-backup before mutation
- Works with `--local` flag

## UX polish

### TUI layout
- Review all screens for spacing/alignment issues
- Ensure long names with brackets/spaces display correctly without truncation
- Adjust column widths in dashboard table for real-world data

### --help with examples
Add `after_help` to clap with usage examples:
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
- `backup_config()` uses `exec_in_container("cp /opt/amnezia/xray/server.json /opt/amnezia/xray/server.json.bak")`
- Timestamped backups use `exec_in_container("cp ... server.json.$(date +%Y%m%d-%H%M%S).bak")`
- `--restore` lists backups via `exec_in_container("ls -t /opt/amnezia/xray/*.bak")`
- `--rename-user` modifies both clientsTable and server.json, then restarts container
- All new CLI commands follow existing pattern: add flag to `Cli` struct, implement async handler in `main.rs`, update test Cli instances

## Out of scope
- Multiple backup retention policy (keep N backups) — v0.2.0
- Undo last operation — v0.2.0
- Export/import config to local machine — v0.2.0
