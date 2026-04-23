# Inventory

## Topology

```
Clients (RU)
    ↓ XHTTP+Reality, SNI=www.sberbank.ru
    ↓
yc-vm  81.26.190.206:443   ←── bridge (Yandex Cloud, Ubuntu 24.04)
    │  routing: geoip:ru → direct, else → foreign-egress
    ↓ XHTTP+Reality, SNI=yuriy-vps.duckdns.org, dest 127.0.0.1:9443 (nginx self-steal)
    ↓
vps-vpn 103.231.72.109:8444 ←── egress (Ubuntu 24.04)
    │  outbound: freedom
    ↓
Internet
```

## Hosts

### `yc-vm` (bridge) — Yandex Cloud, Moscow
- **Public IP**: `81.26.190.206` (static)
- **OS**: Ubuntu 24.04
- **SSH user**: `yuriy` (sudo NOPASSWD)
- **xray**: native systemd service, binary `/usr/local/bin/xray`, version 26.3.27+
- **Config**: `/usr/local/etc/xray/config.json`
- **Logs**: `/var/log/xray/access.log`, `/var/log/xray/error.log`
- **Sidecar**: `/usr/local/etc/xray/reality-public-key` (plain-text pubkey, used to regenerate URLs without re-running `xray x25519`)
- **Setup notes**: `~/bridge-setup.env` — captures secrets at setup time (private key, short id, path, user UUIDs); mode 600

### `vps-vpn` (egress + legacy services) — foreign VPS
- **Public IP**: `103.231.72.109`
- **OS**: Ubuntu 24.04
- **SSH user**: `root`
- **Ports in use**:
  - `22` — SSH
  - `80` — (idle, used only during `certbot renew`)
  - `443` — **legacy Amnezia Docker container `amnezia-xray`** (do not touch)
  - `8443` — **`mtproxymax` Telegram proxy** (do not touch)
  - `8444` — xray-egress, XHTTP+Reality inbound, bridge connects here
  - `9443` — nginx (127.0.0.1 only), self-steal target for Reality
- **xray**: native systemd, same binary path as bridge
- **nginx**: serves a dummy page on `127.0.0.1:9443` with the LE cert, solely to make Reality handshake look legitimate.
- **Config files**:
  - xray: `/usr/local/etc/xray/config.json`
  - nginx vhost: `/etc/nginx/sites-enabled/vpn-selfsteal.conf`
  - LE cert: `/etc/letsencrypt/live/yuriy-vps.duckdns.org/{fullchain,privkey}.pem`
  - Sidecar: `~/egress-setup.env`

## DNS

- **Domain**: `yuriy-vps.duckdns.org` → `103.231.72.109` (A record)
- **DuckDNS token**: known to Yuriy; not stored on servers. Used only for `curl https://www.duckdns.org/update?domains=yuriy-vps&token=<T>&ip=<IP>` during egress migration.

## Users (as of latest known state)

Live on legacy Amnezia (VPS-vpn:443) AND/OR new bridge (yc-vm:443). The bridge's config is always the authoritative list for bridge-users.

Original 14 Amnezia users: `unknown`, `Admin [macOS Tahoe (26.3.1)]`, `Tima`, `Anton`, `Alexander`, `Anna Sestra Oli`, `Masha`, `Kostya`, `test1`, `Natasha`, `sanek`, `router`, `yuriy`, `ivan`, `rita`.

Bridge-only test migrations so far: `ivan`, `rita`, `masha` (as of 2026-04-23). To see the current list: `ssh yc-vm 'sudo jq ".inbounds[0].settings.clients" /usr/local/etc/xray/config.json'`.

## Reality parameters (bridge inbound — seen by clients)

- Network: `xhttp`
- Security: `reality`
- SNI (`serverNames[0]`): `www.sberbank.ru`
- `dest`: `www.sberbank.ru:443`
- xhttp path + shortId + publicKey — per-install, read from running config.

## Telegram bot state

- **Where**: running as a systemd-less background process on `vps-vpn` (PID varies — started March 2026).
- **Command line**: `amnezia-xray-admin --telegram-bot --local --container amnezia-xray --admin-id 775260 --host 103.231.72.109`
- **Backend**: legacy Amnezia Docker container. Does NOT know about the new bridge.
- **Admin**: Telegram user ID `775260` (Yuriy).

## Parallel services not to touch

On `vps-vpn`:
- `amnezia-xray` Docker container on :443 — original VPN, still has users.
- `mtproxymax` on :8443 — Telegram MTProto proxy (unrelated project, hand-installed by Yuriy).

## Backup locations

- Config backups: whenever you edit `config.json`, copy to `config.json.bak-<epoch>` in the same directory. There is no central backup system yet.
- LE certs: managed by certbot at `/etc/letsencrypt/`. Renewal is automatic via the `certbot.timer` systemd unit.
