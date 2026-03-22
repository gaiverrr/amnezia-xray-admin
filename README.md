# amnezia-xray-admin

A hacker-aesthetic TUI dashboard for managing [Amnezia VPN](https://amnezia.org/)'s Xray (VLESS + XTLS-Reality) server.

Connects to your VPS via SSH, talks to the Xray gRPC API for live user management and traffic stats. No container restarts needed.

<!-- TODO: add screenshot -->

## Features

- **User management** - Add and remove users live via Xray gRPC API (no restart required)
- **Traffic stats** - Real-time upload/download stats per user
- **Online status** - See which users are currently connected and from which IPs
- **QR codes** - Generate `vless://` URLs and scannable QR codes for easy sharing
- **CLI commands** - Non-interactive mode for scripting (`--list-users`, `--add-user`, `--delete-user`, `--rename-user`, `--user-url`, `--user-qr`, `--online-status`, `--server-info`)
- **Backup & restore** - Auto-backup before every mutation; manual `--backup` and `--restore` commands
- **Telegram bot** - Manage users via Telegram commands (`/users`, `/add`, `/delete`, `/url`, `/qr`, `/status`)
- **SSH config support** - Use your existing `~/.ssh/config` aliases (e.g. `ssh vps-vpn`)
- **First-run wizard** - Interactive setup on first launch, no manual config editing needed
- **Cyberpunk TUI** - Matrix green on black, because why not

## Installation

### Homebrew (macOS/Linux)

```sh
brew tap gaiverrr/tap
brew install amnezia-xray-admin
```

### Cargo install

Requires [Rust 1.70+](https://rustup.rs/). On Linux, a C compiler (`gcc`, `make`, `perl`) is also needed.

```sh
cargo install --git https://github.com/gaiverrr/amnezia-xray-admin
```

### From source

```sh
git clone https://github.com/gaiverrr/amnezia-xray-admin.git
cd amnezia-xray-admin
cargo build --release
```

The binary will be at `target/release/amnezia-xray-admin`.

## Quick Start

```sh
# First run - starts interactive setup wizard
amnezia-xray-admin

# Or connect using an SSH config alias
amnezia-xray-admin --ssh-host vps-vpn

# Or connect with explicit parameters
amnezia-xray-admin --host 1.2.3.4 --user root --key ~/.ssh/id_ed25519
```

The setup wizard will guide you through connecting to your VPS and configuring the Xray API.

## CLI Commands

All CLI commands connect via SSH, run the operation, print the result, and exit.

```sh
# List users with traffic stats
amnezia-xray-admin --ssh-host vps-vpn --list-users

# Check server: verify API is enabled, print xray version and user count
amnezia-xray-admin --ssh-host vps-vpn --check-server

# Get vless:// URL for a specific user
amnezia-xray-admin --ssh-host vps-vpn --user-url "Alexander"

# Show QR code in terminal for a user's vless:// URL
amnezia-xray-admin --ssh-host vps-vpn --user-qr "Alexander"

# Show which users are currently online and their IPs
amnezia-xray-admin --ssh-host vps-vpn --online-status

# Show server info: xray version, total traffic, user count, API status
amnezia-xray-admin --ssh-host vps-vpn --server-info

# Add a new user (prints name, UUID, and vless:// URL)
amnezia-xray-admin --ssh-host vps-vpn --add-user "Friend"

# Delete a user (interactive confirmation, or use --yes to skip)
amnezia-xray-admin --ssh-host vps-vpn --delete-user "Friend" --yes

# Rename a user (note: resets traffic stats)
amnezia-xray-admin --ssh-host vps-vpn --rename-user "OldName" "NewName"

# Create a timestamped backup of server.json and clientsTable
amnezia-xray-admin --ssh-host vps-vpn --backup

# Restore from the latest backup
amnezia-xray-admin --ssh-host vps-vpn --restore

# Restore from a specific backup by timestamp
amnezia-xray-admin --ssh-host vps-vpn --restore 20260321-143000
```

Use `--local` to run directly on the VPS (uses `docker exec` instead of SSH):

```sh
amnezia-xray-admin --local --container amnezia-xray --list-users
```

## TUI Keybindings

| Key | Action |
|-----|--------|
| `j` / `k` or arrows | Navigate user list |
| `Enter` | View user details |
| `a` | Add new user |
| `d` | Delete user (with confirmation) |
| `r` | Refresh stats |
| `t` | Setup Telegram bot |
| `q` | Quit / Go back |
| `c` | Copy vless:// URL (in user detail) |
| `Esc` | Close dialog / Go back |

## Telegram Bot

Manage your VPN users from Telegram. The bot runs as a Docker container on your VPS and communicates with the xray container directly.

### Bot commands

| Command | Description |
|---------|-------------|
| `/start` | Welcome message (admin only) |
| `/help` | Show available commands |
| `/users` | List users with traffic stats |
| `/status` | Server info + online users |
| `/add <name>` | Add user, get QR code (without name: shows usage hint) |
| `/delete [name]` | Delete user with confirmation (without name: shows user list as buttons) |
| `/url [name]` | Get vless:// URL (without name: shows user list as buttons) |
| `/qr [name]` | Get QR code as image (without name: shows user list as buttons) |

### Setup via TUI

Press `t` on the dashboard to open the Telegram Bot setup screen. Follow the instructions to create a bot with @BotFather and deploy it to your VPS.

### Setup via CLI

```sh
# Deploy bot to VPS (connects via SSH, pulls Docker image, starts container)
amnezia-xray-admin --ssh-host vps-vpn --deploy-bot --telegram-token "123456:ABC..." --admin-id 123456789
```

The `--admin-id` flag sets your Telegram user ID as the bot admin. To find your ID, send `/start` to [@userinfobot](https://t.me/userinfobot) on Telegram. Only the admin can use bot commands; all other users get "Access denied".

## Configuration

Config is stored at `~/.config/amnezia-xray-admin/config.toml` and is created automatically by the setup wizard.

```toml
# SSH connection
host = "1.2.3.4"
port = 22
user = "root"
key_path = "/home/user/.ssh/id_ed25519"

# Or use an SSH config alias instead of host/port/user/key
# ssh_config_host = "vps-vpn"

# Docker container running Xray (default: amnezia-xray)
container_name = "amnezia-xray"

# Telegram bot (optional)
# telegram_token = "123456:ABC..."
# admin_id = 123456789  # Your Telegram user ID
```

CLI arguments override config file values. Run `amnezia-xray-admin --help` for all options.

## Prerequisites

- A VPS running the **amnezia-xray** Docker container
- SSH access to the VPS (key-based auth or ssh-agent)
- Xray configured with VLESS + XTLS-Reality

The tool will automatically enable the Xray gRPC API on first connection if it's not already configured.

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Run tests before submitting (`cargo test && cargo clippy && cargo fmt --check`)
4. Open a pull request

### How to release

1. Update version in `Cargo.toml` and add a section to `CHANGELOG.md`
2. Run tests: `cargo test && cargo clippy && cargo fmt --check`
3. Commit: `git commit -am "chore: bump version to v0.X.0"`
4. Tag: `git tag -a v0.X.0 -m "v0.X.0"`
5. Push: `git push origin main --tags`
6. GitHub Actions builds release binaries automatically and creates a GitHub Release
7. Update the Homebrew formula in [gaiverrr/homebrew-tap](https://github.com/gaiverrr/homebrew-tap) with the new tarball URL and SHA256

## License

MIT - see [LICENSE](LICENSE) for details.
