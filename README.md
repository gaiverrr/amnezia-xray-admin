# amnezia-xray-admin

A hacker-aesthetic TUI dashboard for managing [Amnezia VPN](https://amnezia.org/)'s Xray (VLESS + XTLS-Reality) server.

Connects to your VPS via SSH, talks to the Xray gRPC API for live user management and traffic stats. No container restarts needed.

<!-- TODO: add screenshot -->

## Features

- **User management** - Add and remove users live via Xray gRPC API (no restart required)
- **Traffic stats** - Real-time upload/download stats per user
- **Online status** - See which users are currently connected and from which IPs
- **QR codes** - Generate `vless://` URLs and scannable QR codes for easy sharing
- **SSH config support** - Use your existing `~/.ssh/config` aliases (e.g. `ssh vps-vpn`)
- **First-run wizard** - Interactive setup on first launch, no manual config editing needed
- **Cyberpunk TUI** - Matrix green on black, because why not

## Installation

### From source

Requires [Rust 1.70+](https://rustup.rs/).

```sh
git clone https://github.com/AmneziaVPN/amnezia-xray-admin.git
cd amnezia-xray-admin
cargo build --release
```

The binary will be at `target/release/amnezia-xray-admin`.

## Usage

```sh
# First run - starts setup wizard
amnezia-xray-admin

# Connect using an SSH config alias
amnezia-xray-admin --ssh-host vps-vpn

# Connect with explicit parameters
amnezia-xray-admin --host 1.2.3.4 --user root --key ~/.ssh/id_ed25519
```

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| `j` / `k` or arrows | Navigate user list |
| `Enter` | View user details |
| `a` | Add new user |
| `d` | Delete user (with confirmation) |
| `r` | Refresh stats |
| `q` | Quit / Go back |
| `c` | Copy vless:// URL (in user detail) |
| `Esc` | Close dialog / Go back |

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
```

CLI arguments override config file values. Run `amnezia-xray-admin --help` for all options.

## Prerequisites

- A VPS running the **amnezia-xray** Docker container
- SSH access to the VPS (key-based auth or ssh-agent)
- Xray configured with VLESS + XTLS-Reality

The tool will automatically enable the Xray gRPC API on first connection if it's not already configured.

## License

MIT - see [LICENSE](LICENSE) for details.
