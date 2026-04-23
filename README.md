# amnezia-xray-admin

Personal CLI + Telegram bot for running a double-hop Xray VPN
(RU bridge → foreign egress) for yourself and a few friends.

This is a hobby project, maintained by [@gaiverrr](https://github.com/gaiverrr)
for his own VPN infrastructure. Not intended as a product; use at your own risk
if you want.

## What it does

- Manage Xray users on a native-systemd xray host (no Amnezia Docker).
- Add / remove users, generate `vless://` URLs with QR codes.
- Run as a Telegram bot for day-to-day ops.
- Ship as a native binary via Homebrew.

## Install

```sh
brew install gaiverrr/tap/amnezia-xray-admin
```

## Use

```sh
# List users on the bridge
amnezia-xray-admin --host <bridge-ssh-alias> --list-users

# Add a user, print URL + ASCII QR
amnezia-xray-admin --host <bridge-ssh-alias> --add-user Alice

# Generate URL for existing user
amnezia-xray-admin --host <bridge-ssh-alias> --user-url Alice

# Run the Telegram bot locally on the bridge host
amnezia-xray-admin --telegram-bot --local --admin-id 123456 --host <bridge-ip>
```

`--host` accepts either an SSH alias from `~/.ssh/config` or a literal
hostname/IP. Use `--local` when the binary is running directly on the bridge
(no SSH, just talks to `/usr/local/etc/xray/config.json`).

See `CLAUDE.md` for details on current infrastructure and architecture.

Operational runbook for Claude Code users: `.claude/skills/amnezia-ops/`.

## License

MIT — see [LICENSE](LICENSE).
