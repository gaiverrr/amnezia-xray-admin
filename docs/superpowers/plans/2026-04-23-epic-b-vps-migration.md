# Epic B + C Implementation Plan — VPS Migration & Telegram Bot Upgrade

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship two stateless Rust subcommands (`migrate-bridge`, `migrate-egress`) that provision fresh VPSes via SSH and atomically cut over, plus update the existing Telegram bot to drive the new bridge's native xray (no Docker) and return URL+QR by default on user creation.

**Architecture:** New `src/migrate/` module (4 files: `mod.rs`, `bridge.rs`, `egress.rs`, `install.rs`). New `src/native/` module for the "native-host xray" backend path — mirrors `src/xray/` but targets `/usr/local/etc/xray/config.json` directly without Docker wrapping. Bot receives one new switch (`--bridge`) routing it through the native path instead of Amnezia. Existing Amnezia code paths remain untouched (backward compat during transition).

**Tech Stack:** Rust 2021, clap derive, russh, teloxide, qrcode, serde_json. No new crates.

**Spec:** `docs/superpowers/specs/2026-04-23-epic-b-vps-migration-design.md`.

---

## File Structure

**Create:**
- `src/migrate/mod.rs` — clap subcommand dispatch, shared prompts, confirmation helpers.
- `src/migrate/install.rs` — apt install, xray installer (bash script runner), systemd helpers.
- `src/migrate/bridge.rs` — `migrate-bridge` flow (4 phases + optional phase 5 bot deploy).
- `src/migrate/egress.rs` — `migrate-egress` flow (7 phases).
- `src/native/mod.rs` — module root for native-host xray operations.
- `src/native/backend.rs` — `NativeBackend`: `XrayBackend` impl that SSHs and runs commands directly (no `docker exec` wrapping).
- `src/native/client.rs` — `NativeXrayClient`: list/add/remove users by editing `/usr/local/etc/xray/config.json` directly + `xray api` for stats.
- `src/native/config_render.rs` — pure functions that render bridge/egress `config.json` templates.
- `src/native/url.rs` — XHTTP+Reality URL + QR renderer (distinct from existing Vision URL in `xray/client.rs`).
- `tests/fixtures/bridge-config-sample.json` — snapshot fixture for tests.
- `tests/fixtures/egress-config-sample.json` — snapshot fixture for tests.

**Modify:**
- `src/config.rs` — add fields to `Cli`: `migrate_bridge`, `migrate_egress`, `new_ssh`, `bridge_ssh`, `telegram_token_opt`, `admin_id_opt`, `duckdns_token`, `dry_run`, `yes`, `skip_old`, `bridge` (bot mode flag).
- `src/main.rs` — dispatch new subcommands, thread `--bridge` flag through bot setup and `--add-user`.
- `src/lib.rs` — `pub mod migrate;` and `pub mod native;`.
- `src/telegram.rs` — branch on `Config::bridge` flag: use `NativeXrayClient` path; `/add` handler replies with URL **and** QR image.
- `src/xray/client.rs` — **leave untouched** except: add `VlessUrlParams::xhttp_path: Option<String>` field and branch URL format accordingly (no separate function).
- `CHANGELOG.md` — entry under "Unreleased".
- `CLAUDE.md` — document new subcommands and bot `--bridge` switch.

---

## Conventions for all tasks

- Use `crate::error::{AppError, Result}` everywhere; no `unwrap()` in non-test code.
- All SSH commands go through `SshSession::exec_command` (existing; `ssh.rs:378`).
- Config rendering functions are **pure**: input → String, no side effects. Easy to snapshot-test.
- Commit after each task (typically one commit per task) with imperative message starting with `feat(migrate):` or `feat(native):` or `test(...):`.
- Run `cargo test && cargo clippy -- -D warnings && cargo fmt --check` before each commit.

---

## Phase 0 — Scaffolding

### Task 0.1: Create empty module tree

**Files:**
- Create: `src/migrate/mod.rs`
- Create: `src/migrate/install.rs`
- Create: `src/migrate/bridge.rs`
- Create: `src/migrate/egress.rs`
- Create: `src/native/mod.rs`
- Create: `src/native/backend.rs`
- Create: `src/native/client.rs`
- Create: `src/native/config_render.rs`
- Create: `src/native/url.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create empty files with doc comments**

Each new file gets a minimal doc comment so `cargo build` passes:

```rust
// src/migrate/mod.rs
//! VPS migration subcommands (`migrate-bridge`, `migrate-egress`).

pub mod bridge;
pub mod egress;
pub mod install;
```

```rust
// src/migrate/bridge.rs
//! `migrate-bridge` subcommand — provision a fresh RU bridge and cut over.
```

```rust
// src/migrate/egress.rs
//! `migrate-egress` subcommand — provision a fresh foreign egress and cut over.
```

```rust
// src/migrate/install.rs
//! Shared install primitives: apt, xray-install.sh, systemd helpers.
```

```rust
// src/native/mod.rs
//! Native-host xray operations (bridge/egress without Docker wrapping).

pub mod backend;
pub mod client;
pub mod config_render;
pub mod url;
```

```rust
// src/native/backend.rs
//! `NativeBackend`: XrayBackend impl for native-systemd xray (no Docker).
```

```rust
// src/native/client.rs
//! `NativeXrayClient`: user mgmt by editing server.json directly + xray api for stats.
```

```rust
// src/native/config_render.rs
//! Pure config.json template rendering (bridge inbound, egress inbound+outbound, routing).
```

```rust
// src/native/url.rs
//! XHTTP+Reality vless:// URL and QR code rendering.
```

- [ ] **Step 2: Wire into `src/lib.rs`**

```rust
// src/lib.rs — add at end of existing mod list:
pub mod migrate;
pub mod native;
```

- [ ] **Step 3: Verify build**

Run: `cargo build 2>&1 | tail -5`
Expected: success, no warnings about unused modules.

- [ ] **Step 4: Commit**

```bash
git add src/migrate/ src/native/ src/lib.rs
git commit -m "chore: scaffold migrate and native modules"
```

---

### Task 0.2: Add CLI flags for new subcommands

**Files:**
- Modify: `src/config.rs` (`Cli` struct)

- [ ] **Step 1: Add flags to `Cli` struct**

Find the `Cli` struct in `src/config.rs`. Add fields (preserve existing fields; add these in alphabetical-ish grouping):

```rust
/// Run `migrate-bridge` subcommand.
#[arg(long)]
pub migrate_bridge: bool,

/// Run `migrate-egress` subcommand.
#[arg(long)]
pub migrate_egress: bool,

/// SSH alias of the NEW target VPS (for migrate commands).
#[arg(long)]
pub new_ssh: Option<String>,

/// SSH alias of the BRIDGE (for migrate-egress).
#[arg(long)]
pub bridge_ssh: Option<String>,

/// DuckDNS token for automated DNS update in migrate-egress.
#[arg(long, env = "DUCKDNS_TOKEN")]
pub duckdns_token: Option<String>,

/// Dry run — print the plan without executing side effects.
#[arg(long)]
pub dry_run: bool,

/// Skip all confirmation prompts.
#[arg(long)]
pub yes: bool,

/// Skip stopping the old host in migrate-egress (if old is unreachable).
#[arg(long)]
pub skip_old: bool,

/// Bot operates against a native-xray bridge (not Amnezia Docker).
#[arg(long)]
pub bridge: bool,
```

Note: existing `--telegram-token` / `--admin-id` flags reused for `migrate-bridge --telegram-token X --admin-id Y` (opt-in bot redeploy).

- [ ] **Step 2: Verify clap compiles**

Run: `cargo build 2>&1 | tail -3`
Expected: success.

- [ ] **Step 3: Verify help output**

Run: `cargo run --quiet -- --help 2>&1 | grep -E 'migrate|new-ssh|bridge-ssh'`
Expected: new flags listed.

- [ ] **Step 4: Commit**

```bash
git add src/config.rs
git commit -m "feat(cli): add flags for migrate-bridge and migrate-egress"
```

---

## Phase 1 — Pure config rendering (test-first)

### Task 1.1: Bridge config render function

**Files:**
- Modify: `src/native/config_render.rs`
- Create: `tests/fixtures/bridge-config-sample.json`

- [ ] **Step 1: Create snapshot fixture**

Write `tests/fixtures/bridge-config-sample.json` with the EXACT expected output for a known input:

```json
{
  "log": {
    "loglevel": "warning",
    "access": "/var/log/xray/access.log",
    "error": "/var/log/xray/error.log"
  },
  "inbounds": [
    {
      "listen": "0.0.0.0",
      "port": 443,
      "tag": "client-in",
      "protocol": "vless",
      "settings": {
        "clients": [
          {"id": "00000000-0000-0000-0000-000000000001", "email": "alice@vpn"},
          {"id": "00000000-0000-0000-0000-000000000002", "email": "bob@vpn"}
        ],
        "decryption": "none"
      },
      "streamSettings": {
        "network": "xhttp",
        "security": "reality",
        "xhttpSettings": {"path": "/testpath"},
        "realitySettings": {
          "dest": "www.sberbank.ru:443",
          "serverNames": ["www.sberbank.ru"],
          "privateKey": "TEST_PRIVATE_KEY",
          "shortIds": ["TESTSID"]
        }
      }
    }
  ],
  "outbounds": [
    {
      "tag": "foreign-egress",
      "protocol": "vless",
      "settings": {
        "vnext": [{
          "address": "1.2.3.4",
          "port": 8444,
          "users": [{"id": "00000000-0000-0000-0000-000000000009", "encryption": "none"}]
        }]
      },
      "streamSettings": {
        "network": "xhttp",
        "security": "reality",
        "xhttpSettings": {"path": "/egresspath"},
        "realitySettings": {
          "fingerprint": "chrome",
          "serverName": "example.duckdns.org",
          "publicKey": "EGRESS_PUBLIC",
          "shortId": "EGRESSSID"
        }
      }
    },
    {"protocol": "freedom", "tag": "direct"},
    {"protocol": "blackhole", "tag": "block"}
  ],
  "routing": {
    "domainStrategy": "IPIfNonMatch",
    "rules": [
      {"type": "field", "ip": ["geoip:private"], "outboundTag": "direct"},
      {"type": "field", "domain": ["geosite:category-ru"], "outboundTag": "direct"},
      {"type": "field", "ip": ["geoip:ru"], "outboundTag": "direct"},
      {"type": "field", "network": "tcp,udp", "outboundTag": "foreign-egress"}
    ]
  }
}
```

- [ ] **Step 2: Write failing test in `src/native/config_render.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_config_matches_snapshot() {
        let input = BridgeConfigInput {
            clients: vec![
                ClientEntry { uuid: "00000000-0000-0000-0000-000000000001".into(), email: "alice@vpn".into() },
                ClientEntry { uuid: "00000000-0000-0000-0000-000000000002".into(), email: "bob@vpn".into() },
            ],
            reality_private_key: "TEST_PRIVATE_KEY".into(),
            reality_short_id: "TESTSID".into(),
            xhttp_path: "/testpath".into(),
            sni: "www.sberbank.ru".into(),
            egress_outbound: EgressOutbound {
                address: "1.2.3.4".into(),
                port: 8444,
                bridge_uuid: "00000000-0000-0000-0000-000000000009".into(),
                xhttp_path: "/egresspath".into(),
                reality_public_key: "EGRESS_PUBLIC".into(),
                reality_short_id: "EGRESSSID".into(),
                server_name: "example.duckdns.org".into(),
            },
        };

        let rendered = render_bridge_config(&input).unwrap();
        let actual: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let expected: serde_json::Value = serde_json::from_str(
            include_str!("../../tests/fixtures/bridge-config-sample.json")
        ).unwrap();

        assert_eq!(actual, expected);
    }
}
```

- [ ] **Step 3: Run test to verify it fails (undefined types)**

Run: `cargo test --lib native::config_render 2>&1 | tail -10`
Expected: compile errors — `BridgeConfigInput`, `ClientEntry`, `EgressOutbound`, `render_bridge_config` not defined.

- [ ] **Step 4: Implement types and function**

In `src/native/config_render.rs`:

```rust
use crate::error::{AppError, Result};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct ClientEntry {
    pub uuid: String,
    pub email: String,
}

#[derive(Debug, Clone)]
pub struct EgressOutbound {
    pub address: String,
    pub port: u16,
    pub bridge_uuid: String,
    pub xhttp_path: String,
    pub reality_public_key: String,
    pub reality_short_id: String,
    pub server_name: String,
}

#[derive(Debug, Clone)]
pub struct BridgeConfigInput {
    pub clients: Vec<ClientEntry>,
    pub reality_private_key: String,
    pub reality_short_id: String,
    pub xhttp_path: String,
    pub sni: String,
    pub egress_outbound: EgressOutbound,
}

pub fn render_bridge_config(input: &BridgeConfigInput) -> Result<String> {
    let value = serde_json::json!({
        "log": {
            "loglevel": "warning",
            "access": "/var/log/xray/access.log",
            "error": "/var/log/xray/error.log"
        },
        "inbounds": [{
            "listen": "0.0.0.0",
            "port": 443,
            "tag": "client-in",
            "protocol": "vless",
            "settings": {
                "clients": input.clients.iter().map(|c| serde_json::json!({
                    "id": c.uuid,
                    "email": c.email
                })).collect::<Vec<_>>(),
                "decryption": "none"
            },
            "streamSettings": {
                "network": "xhttp",
                "security": "reality",
                "xhttpSettings": {"path": input.xhttp_path},
                "realitySettings": {
                    "dest": format!("{}:443", input.sni),
                    "serverNames": [input.sni],
                    "privateKey": input.reality_private_key,
                    "shortIds": [input.reality_short_id]
                }
            }
        }],
        "outbounds": [
            {
                "tag": "foreign-egress",
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "address": input.egress_outbound.address,
                        "port": input.egress_outbound.port,
                        "users": [{
                            "id": input.egress_outbound.bridge_uuid,
                            "encryption": "none"
                        }]
                    }]
                },
                "streamSettings": {
                    "network": "xhttp",
                    "security": "reality",
                    "xhttpSettings": {"path": input.egress_outbound.xhttp_path},
                    "realitySettings": {
                        "fingerprint": "chrome",
                        "serverName": input.egress_outbound.server_name,
                        "publicKey": input.egress_outbound.reality_public_key,
                        "shortId": input.egress_outbound.reality_short_id
                    }
                }
            },
            {"protocol": "freedom", "tag": "direct"},
            {"protocol": "blackhole", "tag": "block"}
        ],
        "routing": {
            "domainStrategy": "IPIfNonMatch",
            "rules": [
                {"type": "field", "ip": ["geoip:private"], "outboundTag": "direct"},
                {"type": "field", "domain": ["geosite:category-ru"], "outboundTag": "direct"},
                {"type": "field", "ip": ["geoip:ru"], "outboundTag": "direct"},
                {"type": "field", "network": "tcp,udp", "outboundTag": "foreign-egress"}
            ]
        }
    });
    serde_json::to_string_pretty(&value).map_err(|e| AppError::Config(format!("render bridge config: {e}")))
}
```

- [ ] **Step 5: Run test to verify PASS**

Run: `cargo test --lib native::config_render`
Expected: 1 passed.

- [ ] **Step 6: Commit**

```bash
git add src/native/config_render.rs tests/fixtures/bridge-config-sample.json
git commit -m "feat(native): render_bridge_config with snapshot test"
```

---

### Task 1.2: Egress config render function

**Files:**
- Modify: `src/native/config_render.rs`
- Create: `tests/fixtures/egress-config-sample.json`

- [ ] **Step 1: Create fixture** `tests/fixtures/egress-config-sample.json`:

```json
{
  "log": {
    "loglevel": "warning",
    "access": "/var/log/xray/access.log",
    "error": "/var/log/xray/error.log"
  },
  "inbounds": [
    {
      "listen": "0.0.0.0",
      "port": 8444,
      "tag": "bridge-in",
      "protocol": "vless",
      "settings": {
        "clients": [
          {"id": "00000000-0000-0000-0000-000000000009", "email": "bridge@vpn"}
        ],
        "decryption": "none"
      },
      "streamSettings": {
        "network": "xhttp",
        "security": "reality",
        "xhttpSettings": {"path": "/egresspath"},
        "realitySettings": {
          "dest": "127.0.0.1:9443",
          "serverNames": ["example.duckdns.org"],
          "privateKey": "EGRESS_PRIVATE",
          "shortIds": ["EGRESSSID"]
        }
      }
    }
  ],
  "outbounds": [
    {"protocol": "freedom", "tag": "direct"}
  ]
}
```

- [ ] **Step 2: Write failing test**

Append to `src/native/config_render.rs` tests module:

```rust
#[test]
fn egress_config_matches_snapshot() {
    let input = EgressConfigInput {
        bridge_uuid: "00000000-0000-0000-0000-000000000009".into(),
        port: 8444,
        xhttp_path: "/egresspath".into(),
        reality_private_key: "EGRESS_PRIVATE".into(),
        reality_short_id: "EGRESSSID".into(),
        domain: "example.duckdns.org".into(),
        nginx_port: 9443,
    };
    let rendered = render_egress_config(&input).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    let expected: serde_json::Value = serde_json::from_str(
        include_str!("../../tests/fixtures/egress-config-sample.json")
    ).unwrap();
    assert_eq!(actual, expected);
}
```

- [ ] **Step 3: Run test, verify fails**

Run: `cargo test --lib native::config_render::tests::egress 2>&1 | tail -5`
Expected: compile error — types/function not defined.

- [ ] **Step 4: Implement**

Add to `src/native/config_render.rs`:

```rust
#[derive(Debug, Clone)]
pub struct EgressConfigInput {
    pub bridge_uuid: String,
    pub port: u16,
    pub xhttp_path: String,
    pub reality_private_key: String,
    pub reality_short_id: String,
    pub domain: String,
    pub nginx_port: u16,
}

pub fn render_egress_config(input: &EgressConfigInput) -> Result<String> {
    let value = serde_json::json!({
        "log": {
            "loglevel": "warning",
            "access": "/var/log/xray/access.log",
            "error": "/var/log/xray/error.log"
        },
        "inbounds": [{
            "listen": "0.0.0.0",
            "port": input.port,
            "tag": "bridge-in",
            "protocol": "vless",
            "settings": {
                "clients": [{
                    "id": input.bridge_uuid,
                    "email": "bridge@vpn"
                }],
                "decryption": "none"
            },
            "streamSettings": {
                "network": "xhttp",
                "security": "reality",
                "xhttpSettings": {"path": input.xhttp_path},
                "realitySettings": {
                    "dest": format!("127.0.0.1:{}", input.nginx_port),
                    "serverNames": [input.domain],
                    "privateKey": input.reality_private_key,
                    "shortIds": [input.reality_short_id]
                }
            }
        }],
        "outbounds": [{"protocol": "freedom", "tag": "direct"}]
    });
    serde_json::to_string_pretty(&value).map_err(|e| AppError::Config(format!("render egress config: {e}")))
}
```

- [ ] **Step 5: Run test, verify PASS**

Run: `cargo test --lib native::config_render`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add src/native/config_render.rs tests/fixtures/egress-config-sample.json
git commit -m "feat(native): render_egress_config with snapshot test"
```

---

### Task 1.3: XHTTP vless URL + QR rendering

**Files:**
- Modify: `src/native/url.rs`

- [ ] **Step 1: Write failing test**

```rust
// src/native/url.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xhttp_url_format_matches_expected() {
        let params = XhttpUrlParams {
            uuid: "00000000-0000-0000-0000-000000000001".into(),
            host: "1.2.3.4".into(),
            port: 443,
            path: "/testpath".into(),
            sni: "www.sberbank.ru".into(),
            public_key: "TEST_PUB".into(),
            short_id: "TESTSID".into(),
            name: "alice".into(),
        };
        let url = render_xhttp_url(&params);
        assert_eq!(
            url,
            "vless://00000000-0000-0000-0000-000000000001@1.2.3.4:443?encryption=none&type=xhttp&path=%2Ftestpath&security=reality&sni=www.sberbank.ru&fp=chrome&pbk=TEST_PUB&sid=TESTSID#alice"
        );
    }

    #[test]
    fn qr_produces_valid_png() {
        let url = "vless://test";
        let png = render_qr_png(url).unwrap();
        // PNG magic header
        assert_eq!(&png[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn ascii_qr_non_empty() {
        let ascii = render_qr_ascii("vless://test");
        assert!(ascii.len() > 100);
        assert!(ascii.contains('\n'));
    }
}
```

- [ ] **Step 2: Implement** (add to top of `src/native/url.rs`):

```rust
use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct XhttpUrlParams {
    pub uuid: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub sni: String,
    pub public_key: String,
    pub short_id: String,
    pub name: String,
}

pub fn render_xhttp_url(p: &XhttpUrlParams) -> String {
    let path_encoded = p.path.replace('/', "%2F");
    format!(
        "vless://{uuid}@{host}:{port}?encryption=none&type=xhttp&path={path}&security=reality&sni={sni}&fp=chrome&pbk={pbk}&sid={sid}#{name}",
        uuid = p.uuid,
        host = p.host,
        port = p.port,
        path = path_encoded,
        sni = p.sni,
        pbk = p.public_key,
        sid = p.short_id,
        name = p.name,
    )
}

pub fn render_qr_png(data: &str) -> Result<Vec<u8>> {
    use qrcode::QrCode;
    use qrcode::render::svg;
    // Use qrcode::render::image — need `image` crate; here we hand-roll PNG via resvg? Simpler: use qrcode's Luma bitmap + `image` crate.
    // Instead use qrcode built-in Unicode for ASCII and `render::<Luma<u8>>` for raster.
    let code = QrCode::new(data).map_err(|e| AppError::Config(format!("qr encode: {e}")))?;
    let image = code.render::<image::Luma<u8>>()
        .min_dimensions(200, 200)
        .build();
    let mut buf = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| AppError::Config(format!("qr png write: {e}")))?;
    Ok(buf.into_inner())
}

pub fn render_qr_ascii(data: &str) -> String {
    use qrcode::QrCode;
    use qrcode::render::unicode;
    match QrCode::new(data) {
        Ok(code) => code
            .render::<unicode::Dense1x2>()
            .quiet_zone(true)
            .build(),
        Err(_) => "(qr encoding failed)".to_string(),
    }
}
```

- [ ] **Step 3: Add `image` dependency**

Modify `Cargo.toml` under `[dependencies]`:

```toml
image = { version = "0.25", default-features = false, features = ["png"] }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib native::url`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/native/url.rs Cargo.toml Cargo.lock
git commit -m "feat(native): XHTTP vless URL and QR (PNG + ASCII) rendering"
```

---

### Task 1.4: Parse old-bridge config (extract clients + egress outbound)

**Files:**
- Modify: `src/native/config_render.rs`

- [ ] **Step 1: Write failing test**

Add fixture-based test using existing `tests/fixtures/bridge-config-sample.json`:

```rust
#[test]
fn parse_bridge_config_extracts_clients_and_outbound() {
    let raw = include_str!("../../tests/fixtures/bridge-config-sample.json");
    let parsed = parse_bridge_config(raw).unwrap();

    assert_eq!(parsed.clients.len(), 2);
    assert_eq!(parsed.clients[0].email, "alice@vpn");
    assert_eq!(parsed.egress_outbound.address, "1.2.3.4");
    assert_eq!(parsed.egress_outbound.port, 8444);
    assert_eq!(parsed.egress_outbound.server_name, "example.duckdns.org");
}

#[test]
fn parse_bridge_config_rejects_non_xhttp() {
    let raw = r#"{
        "inbounds": [{
            "protocol": "vless",
            "streamSettings": {"network": "tcp", "security": "reality"}
        }],
        "outbounds": []
    }"#;
    let err = parse_bridge_config(raw).unwrap_err();
    assert!(err.to_string().contains("xhttp"));
}
```

- [ ] **Step 2: Implement**

Add to `src/native/config_render.rs`:

```rust
#[derive(Debug)]
pub struct ParsedBridgeConfig {
    pub clients: Vec<ClientEntry>,
    pub egress_outbound: EgressOutbound,
}

pub fn parse_bridge_config(raw: &str) -> Result<ParsedBridgeConfig> {
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| AppError::Config(format!("parse bridge config: {e}")))?;

    let inbound = v["inbounds"][0].clone();
    let network = inbound["streamSettings"]["network"].as_str().unwrap_or("");
    let security = inbound["streamSettings"]["security"].as_str().unwrap_or("");
    if network != "xhttp" || security != "reality" {
        return Err(AppError::Config(format!(
            "bridge inbound must be network=xhttp security=reality, got network={network} security={security}"
        )));
    }

    let clients = inbound["settings"]["clients"]
        .as_array()
        .ok_or_else(|| AppError::Config("missing clients array".into()))?
        .iter()
        .map(|c| ClientEntry {
            uuid: c["id"].as_str().unwrap_or("").to_string(),
            email: c["email"].as_str().unwrap_or("").to_string(),
        })
        .collect();

    let outbound_raw = v["outbounds"]
        .as_array()
        .and_then(|arr| arr.iter().find(|o| o["tag"] == "foreign-egress"))
        .ok_or_else(|| AppError::Config("foreign-egress outbound not found".into()))?;

    let egress = EgressOutbound {
        address: outbound_raw["settings"]["vnext"][0]["address"].as_str().unwrap_or("").to_string(),
        port: outbound_raw["settings"]["vnext"][0]["port"].as_u64().unwrap_or(0) as u16,
        bridge_uuid: outbound_raw["settings"]["vnext"][0]["users"][0]["id"].as_str().unwrap_or("").to_string(),
        xhttp_path: outbound_raw["streamSettings"]["xhttpSettings"]["path"].as_str().unwrap_or("").to_string(),
        reality_public_key: outbound_raw["streamSettings"]["realitySettings"]["publicKey"].as_str().unwrap_or("").to_string(),
        reality_short_id: outbound_raw["streamSettings"]["realitySettings"]["shortId"].as_str().unwrap_or("").to_string(),
        server_name: outbound_raw["streamSettings"]["realitySettings"]["serverName"].as_str().unwrap_or("").to_string(),
    };

    Ok(ParsedBridgeConfig { clients, egress_outbound: egress })
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib native::config_render`
Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
git add src/native/config_render.rs
git commit -m "feat(native): parse_bridge_config extracts clients and egress outbound"
```

---

## Phase 2 — Shared install helpers (over SSH)

### Task 2.1: apt-install helper with result assertion

**Files:**
- Modify: `src/migrate/install.rs`

- [ ] **Step 1: Write failing unit test using mock XrayBackend**

First, create mock backend in a shared test helper. Add at the bottom of `src/migrate/install.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_trait::XrayBackend;
    use crate::ssh::CommandOutput;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockBackend {
        calls: Mutex<Vec<String>>,
        responses: Mutex<Vec<CommandOutput>>,
    }

    #[async_trait]
    impl XrayBackend for MockBackend {
        async fn exec_in_container(&self, cmd: &str) -> crate::error::Result<CommandOutput> {
            self.exec_on_host(cmd).await
        }
        async fn exec_on_host(&self, cmd: &str) -> crate::error::Result<CommandOutput> {
            self.calls.lock().unwrap().push(cmd.to_string());
            Ok(self.responses.lock().unwrap().remove(0))
        }
        fn container_name(&self) -> &str { "mock" }
        fn hostname(&self) -> &str { "mock.example.com" }
    }

    #[tokio::test]
    async fn apt_install_calls_correct_commands() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput { stdout: "ok".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "ok".into(), stderr: "".into(), exit_code: 0 },
            ]),
        };
        apt_install(&backend, &["nginx", "certbot"]).await.unwrap();
        let calls = backend.calls.lock().unwrap();
        assert!(calls[0].contains("apt-get update"));
        assert!(calls[1].contains("apt-get install"));
        assert!(calls[1].contains("nginx"));
        assert!(calls[1].contains("certbot"));
    }

    #[tokio::test]
    async fn apt_install_fails_on_nonzero_exit() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput { stdout: "".into(), stderr: "E: broken".into(), exit_code: 100 },
            ]),
        };
        let result = apt_install(&backend, &["nginx"]).await;
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests — expect compile fails**

Run: `cargo test --lib migrate::install 2>&1 | tail -5`
Expected: `apt_install` not defined.

- [ ] **Step 3: Implement**

At top of `src/migrate/install.rs`:

```rust
use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};

pub async fn apt_install(backend: &dyn XrayBackend, packages: &[&str]) -> Result<()> {
    let update = backend.exec_on_host(
        "sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq"
    ).await?;
    if !update.success() {
        return Err(AppError::Config(format!(
            "apt-get update failed: {}", update.stderr
        )));
    }
    let install = backend.exec_on_host(&format!(
        "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {}",
        packages.join(" ")
    )).await?;
    if !install.success() {
        return Err(AppError::Config(format!(
            "apt-get install failed: {}", install.stderr
        )));
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests — expect PASS**

Run: `cargo test --lib migrate::install`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/migrate/install.rs
git commit -m "feat(migrate): apt_install helper with mock-backend tests"
```

---

### Task 2.2: xray installer wrapper + version check

**Files:**
- Modify: `src/migrate/install.rs`

- [ ] **Step 1: Add tests**

```rust
#[tokio::test]
async fn install_xray_runs_official_script() {
    let backend = MockBackend {
        calls: Mutex::new(vec![]),
        responses: Mutex::new(vec![
            CommandOutput { stdout: "Xray installed".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "Xray 26.3.27 ...\nA unified platform...".into(), stderr: "".into(), exit_code: 0 },
        ]),
    };
    let version = install_xray(&backend).await.unwrap();
    assert!(version.starts_with("26."));
    let calls = backend.calls.lock().unwrap();
    assert!(calls[0].contains("Xray-install"));
    assert!(calls[1].contains("xray version"));
}

#[tokio::test]
async fn install_xray_rejects_old_version() {
    let backend = MockBackend {
        calls: Mutex::new(vec![]),
        responses: Mutex::new(vec![
            CommandOutput { stdout: "ok".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "Xray 1.8.4 ...".into(), stderr: "".into(), exit_code: 0 },
        ]),
    };
    let err = install_xray(&backend).await.unwrap_err();
    assert!(err.to_string().contains("too old") || err.to_string().contains("1.8"));
}
```

- [ ] **Step 2: Implement**

```rust
pub async fn install_xray(backend: &dyn XrayBackend) -> Result<String> {
    let install_cmd = "sudo bash -c \"$(curl -Ls https://github.com/XTLS/Xray-install/raw/main/install-release.sh)\" @ install";
    let install = backend.exec_on_host(install_cmd).await?;
    if !install.success() {
        return Err(AppError::Config(format!("xray install failed: {}", install.stderr)));
    }

    let version = backend.exec_on_host("xray version 2>&1 | head -1").await?;
    if !version.success() {
        return Err(AppError::Config("xray version check failed".into()));
    }

    // Parse "Xray 26.3.27 (Xray, Penetrates Everything.)"
    let stdout = version.stdout.trim();
    let token = stdout
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| AppError::Config(format!("cannot parse xray version from: {stdout}")))?;
    let major: u32 = token.split('.').next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::Config(format!("cannot parse major version from: {token}")))?;
    if major < 25 {
        return Err(AppError::Config(format!("xray version {token} is too old; need 25+ for XHTTP")));
    }
    Ok(token.to_string())
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib migrate::install`
Expected: 4 passed total.

- [ ] **Step 4: Commit**

```bash
git add src/migrate/install.rs
git commit -m "feat(migrate): install_xray with version check"
```

---

### Task 2.3: Pre-flight checks

**Files:**
- Modify: `src/migrate/install.rs`

- [ ] **Step 1: Write tests**

```rust
#[tokio::test]
async fn preflight_passes_on_healthy_host() {
    let backend = MockBackend {
        calls: Mutex::new(vec![]),
        responses: Mutex::new(vec![
            CommandOutput { stdout: "".into(), stderr: "".into(), exit_code: 0 },        // sudo -n true
            CommandOutput { stdout: "Ubuntu 24.04.4 LTS".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "MemAvailable:    1500000".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "".into(), stderr: "".into(), exit_code: 1 },        // port check: no listener
        ]),
    };
    preflight(&backend, &[443]).await.unwrap();
}

#[tokio::test]
async fn preflight_fails_on_busy_port() {
    let backend = MockBackend {
        calls: Mutex::new(vec![]),
        responses: Mutex::new(vec![
            CommandOutput { stdout: "".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "Ubuntu 24.04.4 LTS".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "MemAvailable:    1500000".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "LISTEN 0 128 *:443 *:* users:((something))".into(), stderr: "".into(), exit_code: 0 },
        ]),
    };
    let err = preflight(&backend, &[443]).await.unwrap_err();
    assert!(err.to_string().contains("443"));
}
```

- [ ] **Step 2: Implement**

```rust
pub async fn preflight(backend: &dyn XrayBackend, required_free_ports: &[u16]) -> Result<()> {
    let sudo = backend.exec_on_host("sudo -n true").await?;
    if !sudo.success() {
        return Err(AppError::Config("sudo requires password — configure NOPASSWD".into()));
    }
    let os = backend.exec_on_host("grep PRETTY_NAME /etc/os-release").await?;
    if !os.success() || !os.stdout.contains("Ubuntu 2") {
        return Err(AppError::Config(format!(
            "unsupported OS (need Ubuntu 22+/24+): {}", os.stdout.trim()
        )));
    }
    let mem = backend.exec_on_host("grep MemAvailable /proc/meminfo").await?;
    let mem_kb: u64 = mem.stdout.split_whitespace().nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if mem_kb < 500_000 {
        return Err(AppError::Config(format!(
            "insufficient memory: {mem_kb} KB available, need ≥ 500_000"
        )));
    }
    for port in required_free_ports {
        let check = backend.exec_on_host(&format!("ss -tln | grep -E \":{port}\\b\" | head -1")).await?;
        if !check.stdout.trim().is_empty() {
            return Err(AppError::Config(format!("port {port} is already in use on new host")));
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib migrate::install`
Expected: 6 passed.

- [ ] **Step 4: Commit**

```bash
git add src/migrate/install.rs
git commit -m "feat(migrate): preflight checks (sudo, OS, mem, ports)"
```

---

### Task 2.4: xray key/secret generator (remote)

**Files:**
- Modify: `src/migrate/install.rs`

- [ ] **Step 1: Tests**

```rust
#[tokio::test]
async fn generate_secrets_parses_output() {
    let backend = MockBackend {
        calls: Mutex::new(vec![]),
        responses: Mutex::new(vec![
            CommandOutput {
                stdout: "PrivateKey: ABC_PRIV\nPassword (PublicKey): ABC_PUB\nHash32: HASH".into(),
                stderr: "".into(),
                exit_code: 0,
            },
            CommandOutput { stdout: "833552e201595cd4".into(), stderr: "".into(), exit_code: 0 },
            CommandOutput { stdout: "0e1fa74ddc24".into(), stderr: "".into(), exit_code: 0 },
        ]),
    };
    let s = generate_secrets(&backend).await.unwrap();
    assert_eq!(s.reality_private, "ABC_PRIV");
    assert_eq!(s.reality_public, "ABC_PUB");
    assert_eq!(s.short_id, "833552e201595cd4");
    assert_eq!(s.path, "/0e1fa74ddc24");
}
```

- [ ] **Step 2: Implement**

```rust
#[derive(Debug, Clone)]
pub struct Secrets {
    pub reality_private: String,
    pub reality_public: String,
    pub short_id: String,
    pub path: String,
}

pub async fn generate_secrets(backend: &dyn XrayBackend) -> Result<Secrets> {
    let keys = backend.exec_on_host("xray x25519").await?;
    if !keys.success() {
        return Err(AppError::Config(format!("xray x25519 failed: {}", keys.stderr)));
    }
    let mut priv_key = String::new();
    let mut pub_key = String::new();
    for line in keys.stdout.lines() {
        if let Some(rest) = line.strip_prefix("PrivateKey:") {
            priv_key = rest.trim().to_string();
        }
        if line.starts_with("Password") {
            if let Some(idx) = line.find(':') {
                pub_key = line[idx + 1..].trim().to_string();
            }
        }
    }
    if priv_key.is_empty() || pub_key.is_empty() {
        return Err(AppError::Config(format!(
            "cannot parse x25519 output: {}", keys.stdout
        )));
    }
    let sid = backend.exec_on_host("openssl rand -hex 8").await?;
    let path = backend.exec_on_host("openssl rand -hex 6").await?;
    Ok(Secrets {
        reality_private: priv_key,
        reality_public: pub_key,
        short_id: sid.stdout.trim().to_string(),
        path: format!("/{}", path.stdout.trim()),
    })
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib migrate::install`
Expected: 7 passed.

- [ ] **Step 4: Commit**

```bash
git add src/migrate/install.rs
git commit -m "feat(migrate): generate_secrets via xray x25519 + openssl"
```

---

### Task 2.5: Write config atomically (sudo tee)

**Files:**
- Modify: `src/migrate/install.rs`

- [ ] **Step 1: Test**

```rust
#[tokio::test]
async fn write_xray_config_uses_base64() {
    let backend = MockBackend {
        calls: Mutex::new(vec![]),
        responses: Mutex::new(vec![
            CommandOutput { stdout: "".into(), stderr: "".into(), exit_code: 0 },
        ]),
    };
    write_xray_config(&backend, "{\"a\":1}").await.unwrap();
    let calls = backend.calls.lock().unwrap();
    assert!(calls[0].contains("base64 -d"));
    assert!(calls[0].contains("/usr/local/etc/xray/config.json"));
}
```

- [ ] **Step 2: Implement**

```rust
use base64::prelude::*;

pub const NATIVE_CONFIG_PATH: &str = "/usr/local/etc/xray/config.json";

pub async fn write_xray_config(backend: &dyn XrayBackend, content: &str) -> Result<()> {
    let encoded = BASE64_STANDARD.encode(content);
    let cmd = format!(
        "echo '{encoded}' | base64 -d | sudo tee {NATIVE_CONFIG_PATH} > /dev/null && sudo chmod 644 {NATIVE_CONFIG_PATH}"
    );
    let out = backend.exec_on_host(&cmd).await?;
    if !out.success() {
        return Err(AppError::Config(format!("write config failed: {}", out.stderr)));
    }
    Ok(())
}

pub async fn restart_xray(backend: &dyn XrayBackend) -> Result<()> {
    let out = backend.exec_on_host("sudo systemctl restart xray").await?;
    if !out.success() {
        return Err(AppError::Config(format!("systemctl restart xray: {}", out.stderr)));
    }
    // Wait briefly and check it's alive
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let status = backend.exec_on_host("sudo systemctl is-active xray").await?;
    if !status.stdout.trim().eq("active") {
        return Err(AppError::Config(format!(
            "xray not active after restart: {}", status.stdout
        )));
    }
    Ok(())
}
```

- [ ] **Step 3: Test**

Run: `cargo test --lib migrate::install`
Expected: 8 passed.

- [ ] **Step 4: Commit**

```bash
git add src/migrate/install.rs
git commit -m "feat(migrate): write_xray_config + restart_xray helpers"
```

---

## Phase 3 — Bridge migration flow

### Task 3.1: `migrate-bridge` module structure and phase 1 (pre-flight + read old)

**Files:**
- Modify: `src/migrate/bridge.rs`

- [ ] **Step 1: Scaffold**

```rust
// src/migrate/bridge.rs
use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use crate::migrate::install;
use crate::native::config_render::{parse_bridge_config, BridgeConfigInput, ClientEntry};

pub struct BridgeMigrateOptions {
    pub yes: bool,
    pub dry_run: bool,
    pub telegram_token: Option<String>,
    pub admin_id: Option<String>,
}

pub struct BridgeMigrationContext {
    pub new_hostname: String,
    pub new_ip: String,
    pub clients: Vec<ClientEntry>,
}

pub async fn run(
    old_backend: &dyn XrayBackend,
    new_backend: &dyn XrayBackend,
    opts: BridgeMigrateOptions,
) -> Result<()> {
    println!("==> Phase 1: pre-flight");
    phase1_preflight(old_backend, new_backend).await?;

    // Further phases added in next tasks.
    Ok(())
}

async fn phase1_preflight(
    old_backend: &dyn XrayBackend,
    new_backend: &dyn XrayBackend,
) -> Result<BridgeMigrationContext> {
    // new host: sudo, OS, mem, port 443 free
    install::preflight(new_backend, &[443]).await?;
    // old host: read config.json
    let out = old_backend
        .exec_on_host("sudo cat /usr/local/etc/xray/config.json")
        .await?;
    if !out.success() {
        return Err(AppError::Config(format!(
            "read old bridge config: {}", out.stderr
        )));
    }
    let parsed = parse_bridge_config(&out.stdout)?;
    // Also probe sberbank.ru from new host
    let probe = new_backend.exec_on_host(
        "curl -sI --max-time 5 --http2 -o /dev/null -w '%{http_version} %{http_code}' https://www.sberbank.ru"
    ).await?;
    if !probe.stdout.contains("2 200") {
        return Err(AppError::Config(format!(
            "sberbank.ru unreachable from new host: '{}'", probe.stdout
        )));
    }
    println!("    found {} clients, sberbank.ru OK", parsed.clients.len());
    Ok(BridgeMigrationContext {
        new_hostname: new_backend.hostname().to_string(),
        new_ip: new_backend.hostname().to_string(), // will refine with resolve
        clients: parsed.clients,
    })
}
```

- [ ] **Step 2: Verify build**

Run: `cargo build 2>&1 | tail -3`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add src/migrate/bridge.rs
git commit -m "feat(migrate): bridge module scaffold with phase 1 pre-flight"
```

---

### Task 3.2: Phase 2 — provision new bridge

- [ ] **Step 1: Extend `phase1_preflight` return to include egress_outbound, then implement phase 2**

```rust
// src/migrate/bridge.rs — augment BridgeMigrationContext:
use crate::native::config_render::EgressOutbound;

pub struct BridgeMigrationContext {
    pub new_hostname: String,
    pub new_ip: String,
    pub clients: Vec<ClientEntry>,
    pub egress_outbound: EgressOutbound,
}

// In phase1_preflight, replace the last constructor with:
Ok(BridgeMigrationContext {
    new_hostname: new_backend.hostname().to_string(),
    new_ip: new_backend.hostname().to_string(),
    clients: parsed.clients,
    egress_outbound: parsed.egress_outbound,
})
```

Add phase 2:

```rust
use crate::migrate::install::Secrets;
use crate::native::config_render::{render_bridge_config, BridgeConfigInput};
use uuid::Uuid;

pub struct BridgeProvisioned {
    pub secrets: Secrets,
    pub sni: String,
    pub new_ip: String,
    pub new_clients: Vec<ClientEntry>,
}

async fn phase2_provision(
    new_backend: &dyn XrayBackend,
    ctx: &BridgeMigrationContext,
) -> Result<BridgeProvisioned> {
    println!("==> Phase 2: provision new bridge");
    install::apt_install(new_backend, &["curl", "jq", "openssl", "ca-certificates"]).await?;
    install::install_xray(new_backend).await?;

    let secrets = install::generate_secrets(new_backend).await?;
    // Regenerate UUIDs for every user.
    let new_clients: Vec<ClientEntry> = ctx.clients.iter().map(|c| ClientEntry {
        uuid: Uuid::new_v4().to_string(),
        email: c.email.clone(),
    }).collect();

    let input = BridgeConfigInput {
        clients: new_clients.clone(),
        reality_private_key: secrets.reality_private.clone(),
        reality_short_id: secrets.short_id.clone(),
        xhttp_path: secrets.path.clone(),
        sni: "www.sberbank.ru".into(),
        egress_outbound: ctx.egress_outbound.clone(),
    };
    let rendered = render_bridge_config(&input)?;
    // Ensure log dir exists
    new_backend.exec_on_host("sudo mkdir -p /var/log/xray").await?;
    install::write_xray_config(new_backend, &rendered).await?;
    install::restart_xray(new_backend).await?;

    // Resolve public IP (so URLs point correctly)
    let ip_out = new_backend.exec_on_host("curl -s --max-time 5 https://ifconfig.me").await?;
    let new_ip = ip_out.stdout.trim().to_string();
    if new_ip.is_empty() {
        return Err(AppError::Config("cannot resolve new bridge public IP".into()));
    }
    Ok(BridgeProvisioned {
        secrets,
        sni: "www.sberbank.ru".to_string(),
        new_ip,
        new_clients,
    })
}
```

Update `run()` to call it:

```rust
pub async fn run(
    old_backend: &dyn XrayBackend,
    new_backend: &dyn XrayBackend,
    opts: BridgeMigrateOptions,
) -> Result<()> {
    println!("==> Phase 1: pre-flight");
    let ctx = phase1_preflight(old_backend, new_backend).await?;
    let provisioned = phase2_provision(new_backend, &ctx).await?;
    println!("    new bridge listening on {}:443", provisioned.new_ip);
    Ok(())
}
```

- [ ] **Step 2: Build passes**

Run: `cargo build 2>&1 | tail -3`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add src/migrate/bridge.rs
git commit -m "feat(migrate): bridge phase 2 — provision new host"
```

---

### Task 3.3: Phase 3 — URL generation and file output

- [ ] **Step 1: Add `tokio::fs` + chrono dep usage (already present), extend run**

```rust
// src/migrate/bridge.rs
use crate::native::url::{render_xhttp_url, render_qr_ascii, XhttpUrlParams};

async fn phase3_urls(
    provisioned: &BridgeProvisioned,
) -> Result<Vec<(String, String)>> {
    println!("==> Phase 3: generate URLs");
    let mut urls = Vec::new();
    for client in &provisioned.new_clients {
        let name = client.email.trim_end_matches("@vpn").to_string();
        let url = render_xhttp_url(&XhttpUrlParams {
            uuid: client.uuid.clone(),
            host: provisioned.new_ip.clone(),
            port: 443,
            path: provisioned.secrets.path.clone(),
            sni: provisioned.sni.clone(),
            public_key: provisioned.secrets.reality_public.clone(),
            short_id: provisioned.secrets.short_id.clone(),
            name: name.clone(),
        });
        urls.push((name, url));
    }

    // Save to file
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("urls-{timestamp}.txt");
    let mut contents = String::new();
    for (name, url) in &urls {
        contents.push_str(&format!("=== {name} ===\n{url}\n\n"));
    }
    tokio::fs::write(&filename, &contents).await.map_err(AppError::Io)?;
    println!("    wrote {} URLs to {}", urls.len(), filename);

    // Print to stdout
    for (name, url) in &urls {
        println!("\n----- {name} -----");
        println!("{url}");
        if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            println!("{}", render_qr_ascii(url));
        }
    }

    Ok(urls)
}
```

Ensure `chrono` is in `Cargo.toml` (it is — used elsewhere; if not, add `chrono = "0.4"`).

Update `run()`:

```rust
let provisioned = phase2_provision(new_backend, &ctx).await?;
let _urls = phase3_urls(&provisioned).await?;
```

- [ ] **Step 2: Build**

Run: `cargo build 2>&1 | tail -3`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add src/migrate/bridge.rs Cargo.toml
git commit -m "feat(migrate): bridge phase 3 — URL file output with ASCII QR"
```

---

### Task 3.4: Phase 4 — cutover with confirmation prompt

- [ ] **Step 1: Add prompt helper to `src/migrate/mod.rs`**

```rust
// src/migrate/mod.rs
pub fn confirm(prompt: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    use std::io::Write;
    print!("{prompt} [y/N]: ");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}
```

- [ ] **Step 2: Phase 4 implementation**

```rust
// src/migrate/bridge.rs
async fn phase4_cutover(
    old_backend: &dyn XrayBackend,
    opts: &BridgeMigrateOptions,
) -> Result<()> {
    if !crate::migrate::confirm("==> URLs delivered. Stop old bridge?", opts.yes) {
        println!("    cutover aborted — old bridge still running");
        return Ok(());
    }
    println!("==> Phase 4: cutover — stopping old bridge");
    let stop = old_backend.exec_on_host("sudo systemctl stop xray && sudo systemctl disable xray").await?;
    if !stop.success() {
        return Err(AppError::Ssh(format!("stop old xray: {}", stop.stderr)));
    }
    // Stop bot if present
    old_backend.exec_on_host("sudo systemctl stop amnezia-xray-bot 2>/dev/null || true").await?;
    println!("    old bridge stopped. VPS may be destroyed in provider UI.");
    Ok(())
}
```

Update `run`:

```rust
let _urls = phase3_urls(&provisioned).await?;
phase4_cutover(old_backend, &opts).await?;
```

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -3`
Expected: success.

- [ ] **Step 4: Commit**

```bash
git add src/migrate/mod.rs src/migrate/bridge.rs
git commit -m "feat(migrate): bridge phase 4 — cutover with confirmation"
```

---

### Task 3.5: Phase 5 — optional bot redeploy

- [ ] **Step 1: Implement bot-deploy helper**

Reuse existing `cli_deploy_bot` logic pattern from `main.rs:122`. The original uses cross-compile + docker image wrap. Simpler: reuse cross-compile but skip docker wrap, install as systemd unit.

```rust
// src/migrate/bridge.rs
async fn phase5_bot_deploy(
    new_backend: &dyn XrayBackend,
    telegram_token: &str,
    admin_id: &str,
) -> Result<()> {
    println!("==> Phase 5: deploy Telegram bot on new bridge");
    // 1. cross-compile locally
    let target = detect_arch(new_backend).await?;
    println!("    detected target arch: {target}");
    let output = tokio::process::Command::new("cross")
        .args(["build", "--release", "--target", &target, "--bin", "amnezia-xray-admin"])
        .output()
        .await
        .map_err(AppError::Io)?;
    if !output.status.success() {
        return Err(AppError::Config(format!(
            "cross-compile failed: {}", String::from_utf8_lossy(&output.stderr)
        )));
    }
    // 2. scp to new host
    let bin_path = format!("target/{target}/release/amnezia-xray-admin");
    let remote = format!("{}:/tmp/amnezia-xray-admin", new_backend.hostname());
    let scp = tokio::process::Command::new("scp")
        .args([&bin_path, &remote])
        .output()
        .await
        .map_err(AppError::Io)?;
    if !scp.status.success() {
        return Err(AppError::Config(format!(
            "scp failed: {}", String::from_utf8_lossy(&scp.stderr)
        )));
    }
    // 3. move + systemd unit + env file
    let unit = format!(r#"[Unit]
Description=amnezia-xray-admin telegram bot
After=network.target xray.service

[Service]
Type=simple
Environment=TELOXIDE_TOKEN={telegram_token}
ExecStart=/usr/local/bin/amnezia-xray-admin --telegram-bot --bridge --admin-id {admin_id}
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
"#);
    let encoded = base64::prelude::BASE64_STANDARD.encode(&unit);
    let install_cmd = format!(
        "sudo mv /tmp/amnezia-xray-admin /usr/local/bin/amnezia-xray-admin && \
         sudo chmod +x /usr/local/bin/amnezia-xray-admin && \
         echo '{encoded}' | base64 -d | sudo tee /etc/systemd/system/amnezia-xray-bot.service > /dev/null && \
         sudo systemctl daemon-reload && \
         sudo systemctl enable --now amnezia-xray-bot"
    );
    let out = new_backend.exec_on_host(&install_cmd).await?;
    if !out.success() {
        return Err(AppError::Config(format!("bot install: {}", out.stderr)));
    }
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    let status = new_backend.exec_on_host("sudo systemctl is-active amnezia-xray-bot").await?;
    if status.stdout.trim() != "active" {
        return Err(AppError::Config(format!(
            "bot not active: {}", status.stdout
        )));
    }
    println!("    bot active on new bridge");
    Ok(())
}

async fn detect_arch(backend: &dyn XrayBackend) -> Result<String> {
    let uname = backend.exec_on_host("uname -m").await?;
    let arch = uname.stdout.trim();
    Ok(match arch {
        "x86_64" => "x86_64-unknown-linux-gnu".to_string(),
        "aarch64" | "arm64" => "aarch64-unknown-linux-gnu".to_string(),
        other => return Err(AppError::Config(format!("unsupported arch: {other}"))),
    })
}
```

Update `run`:

```rust
if let (Some(token), Some(admin)) = (&opts.telegram_token, &opts.admin_id) {
    phase5_bot_deploy(new_backend, token, admin).await?;
}
```

- [ ] **Step 2: Build**

Run: `cargo build 2>&1 | tail -5`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add src/migrate/bridge.rs
git commit -m "feat(migrate): bridge phase 5 — optional bot redeploy"
```

---

## Phase 4 — Egress migration flow

Mirrors bridge but with extra steps for nginx + LE + DuckDNS.

### Task 4.1: Egress phase 1 (pre-flight) + phase 2 (apt install + nginx)

- [ ] **Step 1: Scaffold `src/migrate/egress.rs`**

```rust
use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use crate::migrate::install;
use crate::native::config_render::{render_egress_config, EgressConfigInput};

pub struct EgressMigrateOptions {
    pub yes: bool,
    pub dry_run: bool,
    pub skip_old: bool,
    pub duckdns_token: Option<String>,
}

const EGRESS_PORT: u16 = 8444;
const NGINX_PORT: u16 = 9443;
const DOMAIN: &str = "yuriy-vps.duckdns.org"; // TODO: parameterize later
const LE_EMAIL: &str = "gaiverrr@gmail.com";

pub async fn run(
    old_backend: Option<&dyn XrayBackend>,
    new_backend: &dyn XrayBackend,
    bridge_backend: &dyn XrayBackend,
    opts: EgressMigrateOptions,
) -> Result<()> {
    println!("==> Phase 1: pre-flight");
    install::preflight(new_backend, &[80, EGRESS_PORT, NGINX_PORT]).await?;

    println!("==> Phase 2: install deps + nginx config (not started yet)");
    install::apt_install(new_backend, &[
        "nginx", "certbot", "curl", "jq", "openssl", "ca-certificates",
    ]).await?;
    install::install_xray(new_backend).await?;

    // remove default nginx site
    new_backend.exec_on_host("sudo rm -f /etc/nginx/sites-enabled/default").await?;
    Ok(())
}
```

- [ ] **Step 2: Build**

Run: `cargo build 2>&1 | tail -3`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add src/migrate/egress.rs
git commit -m "feat(migrate): egress phase 1-2 scaffold"
```

---

### Task 4.2: Egress phase 3 — DuckDNS switch-over

- [ ] **Step 1: DuckDNS helper**

Add to `src/migrate/egress.rs`:

```rust
async fn duckdns_update(token: &str, new_ip: &str) -> Result<()> {
    let url = format!(
        "https://www.duckdns.org/update?domains={}&token={}&ip={}",
        DOMAIN.strip_suffix(".duckdns.org").unwrap_or(DOMAIN),
        token,
        new_ip,
    );
    let resp = tokio::process::Command::new("curl")
        .args(["-sS", "--max-time", "10", &url])
        .output()
        .await
        .map_err(AppError::Io)?;
    let body = String::from_utf8_lossy(&resp.stdout);
    if body.trim() != "OK" {
        return Err(AppError::Config(format!("duckdns update: body={body}")));
    }
    Ok(())
}

async fn wait_for_dns(new_ip: &str, timeout_s: u64) -> Result<()> {
    let start = std::time::Instant::now();
    loop {
        let out = tokio::process::Command::new("dig")
            .args(["+short", DOMAIN])
            .output()
            .await
            .map_err(AppError::Io)?;
        let resolved = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if resolved == new_ip {
            return Ok(());
        }
        if start.elapsed().as_secs() > timeout_s {
            return Err(AppError::Config(format!(
                "DNS {DOMAIN} not propagated to {new_ip} after {timeout_s}s (still {resolved})"
            )));
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn phase3_duckdns(
    new_backend: &dyn XrayBackend,
    opts: &EgressMigrateOptions,
) -> Result<String> {
    let ip_out = new_backend.exec_on_host("curl -s --max-time 5 https://ifconfig.me").await?;
    let new_ip = ip_out.stdout.trim().to_string();
    if new_ip.is_empty() {
        return Err(AppError::Config("cannot resolve new egress public IP".into()));
    }
    println!("==> Phase 3: DuckDNS switch-over to {new_ip}");
    if let Some(token) = &opts.duckdns_token {
        duckdns_update(token, &new_ip).await?;
        println!("    DuckDNS updated via API");
    } else {
        println!(
            "    Manually update {DOMAIN} → {new_ip} in the DuckDNS UI, then press Enter..."
        );
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).ok();
    }
    wait_for_dns(&new_ip, 120).await?;
    println!("    DNS propagated");
    Ok(new_ip)
}
```

- [ ] **Step 2: Wire into `run`**

```rust
let new_ip = phase3_duckdns(new_backend, &opts).await?;
```

- [ ] **Step 3: Build + commit**

Run: `cargo build 2>&1 | tail -3`

```bash
git add src/migrate/egress.rs
git commit -m "feat(migrate): egress phase 3 — DuckDNS switch-over + DNS wait"
```

---

### Task 4.3: Egress phase 4 — LE cert + nginx up

- [ ] **Step 1: Implement**

```rust
async fn phase4_cert_nginx(
    new_backend: &dyn XrayBackend,
) -> Result<()> {
    println!("==> Phase 4: Let's Encrypt cert + nginx self-steal");
    let cmd = format!(
        "sudo certbot certonly --standalone -d {DOMAIN} --non-interactive --agree-tos --email {LE_EMAIL}"
    );
    let out = new_backend.exec_on_host(&cmd).await?;
    if !out.success() {
        return Err(AppError::Config(format!("certbot: {}", out.stderr)));
    }

    // nginx config
    let nginx_conf = format!(r#"server {{
    listen 127.0.0.1:{NGINX_PORT} ssl http2;
    server_name {DOMAIN};
    ssl_certificate     /etc/letsencrypt/live/{DOMAIN}/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/{DOMAIN}/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    root /var/www/html;
    index index.html;
    location / {{ try_files $uri $uri/ =404; }}
}}
"#);
    let encoded = base64::prelude::BASE64_STANDARD.encode(nginx_conf);
    let install_nginx = format!(
        "echo '{encoded}' | base64 -d | sudo tee /etc/nginx/sites-available/vpn-selfsteal.conf > /dev/null && \
         sudo ln -sf /etc/nginx/sites-available/vpn-selfsteal.conf /etc/nginx/sites-enabled/vpn-selfsteal.conf && \
         sudo nginx -t && sudo systemctl restart nginx"
    );
    let out = new_backend.exec_on_host(&install_nginx).await?;
    if !out.success() {
        return Err(AppError::Config(format!("nginx setup: {}", out.stderr)));
    }
    // smoke test
    let test = new_backend.exec_on_host(&format!(
        "curl -sI --resolve {DOMAIN}:{NGINX_PORT}:127.0.0.1 https://{DOMAIN}:{NGINX_PORT}/ | head -1"
    )).await?;
    if !test.stdout.contains("200") {
        return Err(AppError::Config(format!("nginx self-steal health check failed: {}", test.stdout)));
    }
    println!("    cert installed, nginx self-steal live on 127.0.0.1:{NGINX_PORT}");
    Ok(())
}
```

- [ ] **Step 2: Wire + commit**

Update `run`:
```rust
phase4_cert_nginx(new_backend).await?;
```

```bash
cargo build 2>&1 | tail -3
git add src/migrate/egress.rs
git commit -m "feat(migrate): egress phase 4 — LE cert + nginx self-steal"
```

---

### Task 4.4: Egress phase 5 — xray config + start

- [ ] **Step 1: Implement**

```rust
struct EgressProvisioned {
    secrets: install::Secrets,
    bridge_uuid: String,
    new_ip: String,
}

async fn phase5_xray(
    new_backend: &dyn XrayBackend,
    new_ip: &str,
) -> Result<EgressProvisioned> {
    println!("==> Phase 5: xray-egress config + start");
    let secrets = install::generate_secrets(new_backend).await?;
    let bridge_uuid = uuid::Uuid::new_v4().to_string();
    let input = EgressConfigInput {
        bridge_uuid: bridge_uuid.clone(),
        port: EGRESS_PORT,
        xhttp_path: secrets.path.clone(),
        reality_private_key: secrets.reality_private.clone(),
        reality_short_id: secrets.short_id.clone(),
        domain: DOMAIN.into(),
        nginx_port: NGINX_PORT,
    };
    let rendered = render_egress_config(&input)?;
    new_backend.exec_on_host("sudo mkdir -p /var/log/xray").await?;
    install::write_xray_config(new_backend, &rendered).await?;
    install::restart_xray(new_backend).await?;
    Ok(EgressProvisioned {
        secrets,
        bridge_uuid,
        new_ip: new_ip.to_string(),
    })
}
```

- [ ] **Step 2: Wire + commit**

```rust
let provisioned = phase5_xray(new_backend, &new_ip).await?;
```

```bash
cargo build 2>&1 | tail -3
git add src/migrate/egress.rs
git commit -m "feat(migrate): egress phase 5 — xray-egress config + start"
```

---

### Task 4.5: Egress phase 6 — bridge outbound cutover

- [ ] **Step 1: Implement**

```rust
async fn phase6_bridge_cutover(
    bridge_backend: &dyn XrayBackend,
    provisioned: &EgressProvisioned,
    yes: bool,
) -> Result<()> {
    if !crate::migrate::confirm("==> Switch bridge to new egress?", yes) {
        return Err(AppError::Config("cutover aborted".into()));
    }
    println!("==> Phase 6: bridge cutover");
    let patch_cmd = format!(r#"sudo jq --arg ip "{ip}" \
      --arg pbk "{pbk}" \
      --arg sid "{sid}" \
      --arg path "{path}" \
      --arg uuid "{uuid}" '
      (.outbounds[] | select(.tag=="foreign-egress") | .settings.vnext[0].address) = $ip |
      (.outbounds[] | select(.tag=="foreign-egress") | .settings.vnext[0].users[0].id) = $uuid |
      (.outbounds[] | select(.tag=="foreign-egress") | .streamSettings.xhttpSettings.path) = $path |
      (.outbounds[] | select(.tag=="foreign-egress") | .streamSettings.realitySettings.publicKey) = $pbk |
      (.outbounds[] | select(.tag=="foreign-egress") | .streamSettings.realitySettings.shortId) = $sid
      ' /usr/local/etc/xray/config.json | sudo tee /usr/local/etc/xray/config.json.new > /dev/null && \
      sudo mv /usr/local/etc/xray/config.json.new /usr/local/etc/xray/config.json && \
      sudo systemctl restart xray"#,
        ip = provisioned.new_ip,
        pbk = provisioned.secrets.reality_public,
        sid = provisioned.secrets.short_id,
        path = provisioned.secrets.path,
        uuid = provisioned.bridge_uuid,
    );
    let out = bridge_backend.exec_on_host(&patch_cmd).await?;
    if !out.success() {
        return Err(AppError::Config(format!("bridge patch+restart: {}", out.stderr)));
    }
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let err_tail = bridge_backend.exec_on_host("sudo tail -5 /var/log/xray/error.log").await?;
    if err_tail.stdout.to_lowercase().contains("reject") {
        return Err(AppError::Config(format!("bridge error log has rejects: {}", err_tail.stdout)));
    }
    println!("    bridge now routes through new egress");
    Ok(())
}
```

- [ ] **Step 2: Wire + commit**

```rust
phase6_bridge_cutover(bridge_backend, &provisioned, opts.yes).await?;
```

```bash
cargo build 2>&1 | tail -3
git add src/migrate/egress.rs
git commit -m "feat(migrate): egress phase 6 — atomic bridge-outbound cutover"
```

---

### Task 4.6: Egress phase 7 — decommission old

- [ ] **Step 1: Implement**

```rust
async fn phase7_decommission(
    old_backend: Option<&dyn XrayBackend>,
    opts: &EgressMigrateOptions,
) -> Result<()> {
    if opts.skip_old {
        println!("==> Phase 7: --skip-old, nothing to do");
        return Ok(());
    }
    let Some(old_backend) = old_backend else {
        println!("==> Phase 7: no --old-ssh provided, skipping");
        return Ok(());
    };
    if !crate::migrate::confirm("==> Stop old egress services?", opts.yes) {
        println!("    skipping decommission");
        return Ok(());
    }
    println!("==> Phase 7: decommission old egress");
    old_backend.exec_on_host(
        "sudo systemctl stop xray 2>/dev/null; sudo systemctl disable xray 2>/dev/null; \
         sudo systemctl stop nginx 2>/dev/null; sudo systemctl disable nginx 2>/dev/null; true"
    ).await?;
    println!("    old egress stopped. VPS may be destroyed.");
    Ok(())
}
```

- [ ] **Step 2: Wire + commit**

```rust
phase7_decommission(old_backend, &opts).await?;
```

```bash
cargo build 2>&1 | tail -3
git add src/migrate/egress.rs
git commit -m "feat(migrate): egress phase 7 — decommission old"
```

---

## Phase 5 — CLI dispatch + main.rs wiring

### Task 5.1: Dispatch new subcommands

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add dispatch block**

In `src/main.rs`, after existing subcommand dispatch (around line 150+), add:

```rust
if cli.migrate_bridge {
    runtime.block_on(cli_migrate_bridge(&config, &cli))?;
    return Ok(());
}
if cli.migrate_egress {
    runtime.block_on(cli_migrate_egress(&config, &cli))?;
    return Ok(());
}
```

Implement at the bottom of `src/main.rs`:

```rust
async fn cli_migrate_bridge(config: &Config, cli: &Cli) -> Result<()> {
    let new_ssh = cli.new_ssh.as_ref()
        .ok_or_else(|| AppError::Config("--new-ssh is required".into()))?;
    let old_ssh = cli.old_ssh.as_ref()
        .ok_or_else(|| AppError::Config("--old-ssh is required".into()))?;

    let new_backend = connect_ssh_native(new_ssh).await?;
    let old_backend = connect_ssh_native(old_ssh).await?;

    let opts = amnezia_xray_admin::migrate::bridge::BridgeMigrateOptions {
        yes: cli.yes,
        dry_run: cli.dry_run,
        telegram_token: cli.telegram_token.clone(),
        admin_id: cli.admin_id.clone(),
    };
    amnezia_xray_admin::migrate::bridge::run(&*old_backend, &*new_backend, opts).await
}

async fn cli_migrate_egress(config: &Config, cli: &Cli) -> Result<()> {
    let new_ssh = cli.new_ssh.as_ref()
        .ok_or_else(|| AppError::Config("--new-ssh is required".into()))?;
    let bridge_ssh = cli.bridge_ssh.as_ref()
        .ok_or_else(|| AppError::Config("--bridge-ssh is required".into()))?;

    let new_backend = connect_ssh_native(new_ssh).await?;
    let bridge_backend = connect_ssh_native(bridge_ssh).await?;
    let old_backend = if let Some(old) = &cli.old_ssh {
        Some(connect_ssh_native(old).await?)
    } else {
        None
    };

    let opts = amnezia_xray_admin::migrate::egress::EgressMigrateOptions {
        yes: cli.yes,
        dry_run: cli.dry_run,
        skip_old: cli.skip_old,
        duckdns_token: cli.duckdns_token.clone(),
    };
    amnezia_xray_admin::migrate::egress::run(
        old_backend.as_deref(),
        &*new_backend,
        &*bridge_backend,
        opts,
    ).await
}

// Reuse existing SSH connection helpers but construct with empty container
// (native mode doesn't wrap with docker exec).
async fn connect_ssh_native(alias: &str) -> Result<Box<dyn XrayBackend>> {
    use amnezia_xray_admin::ssh::{resolve_ssh_host, SshSession};
    use amnezia_xray_admin::backend_trait::SshBackend;

    let host = resolve_ssh_host(alias)
        .ok_or_else(|| AppError::Config(format!("SSH alias '{alias}' not in ~/.ssh/config")))?;
    let session = SshSession::connect(
        &host.hostname,
        host.port,
        &host.user,
        host.identity_file.as_deref(),
        String::new(), // native: no container
    ).await?;
    Ok(Box::new(SshBackend::new(session, host.hostname)))
}
```

Note: `connect_ssh_native` uses empty container name; since `NativeBackend` is planned separately, for now we reuse `SshBackend` with empty container and fix up `exec_in_container` semantics in Phase 6 of the plan. For migrate commands, only `exec_on_host` is called, so `exec_in_container` isn't exercised.

- [ ] **Step 2: Verify old_ssh flag exists**

Check `src/config.rs` for `old_ssh` field on `Cli`. If missing, add:

```rust
#[arg(long)]
pub old_ssh: Option<String>,
```

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -10`
Expected: success (fix visibility issues — may need `pub` on some fields/modules).

- [ ] **Step 4: Commit**

```bash
git add src/main.rs src/config.rs
git commit -m "feat(cli): dispatch migrate-bridge and migrate-egress"
```

---

## Phase 6 — Native bot backend + QR default

### Task 6.1: NativeXrayClient reads/writes server.json clients directly

**Files:**
- Modify: `src/native/client.rs`
- Modify: `src/native/backend.rs`

- [ ] **Step 1: NativeBackend**

```rust
// src/native/backend.rs
use crate::backend_trait::XrayBackend;
use crate::error::Result;
use crate::ssh::{CommandOutput, SshSession};
use async_trait::async_trait;

pub struct NativeBackend {
    session: SshSession,
    hostname: String,
}

impl NativeBackend {
    pub fn new(session: SshSession, hostname: String) -> Self {
        Self { session, hostname }
    }
}

#[async_trait]
impl XrayBackend for NativeBackend {
    async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput> {
        // native: no container wrapping
        self.session.exec_command(cmd).await
    }
    async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput> {
        self.session.exec_command(cmd).await
    }
    fn container_name(&self) -> &str { "" }
    fn hostname(&self) -> &str { &self.hostname }
}
```

- [ ] **Step 2: NativeXrayClient**

```rust
// src/native/client.rs
use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use crate::native::config_render::{parse_bridge_config, ClientEntry, BridgeConfigInput, render_bridge_config};

pub struct NativeXrayClient<'a> {
    pub backend: &'a dyn XrayBackend,
}

impl<'a> NativeXrayClient<'a> {
    pub fn new(backend: &'a dyn XrayBackend) -> Self {
        Self { backend }
    }

    pub async fn list_clients(&self) -> Result<Vec<ClientEntry>> {
        let out = self.backend.exec_on_host("sudo cat /usr/local/etc/xray/config.json").await?;
        if !out.success() {
            return Err(AppError::Config(format!("read config: {}", out.stderr)));
        }
        let parsed = parse_bridge_config(&out.stdout)?;
        Ok(parsed.clients)
    }

    pub async fn add_client(&self, name: &str) -> Result<ClientEntry> {
        let uuid = uuid::Uuid::new_v4().to_string();
        let email = format!("{name}@vpn");
        let add_cmd = format!(
            r#"sudo jq '.inbounds[0].settings.clients += [{{"id":"{uuid}","email":"{email}"}}]' \
            /usr/local/etc/xray/config.json | sudo tee /usr/local/etc/xray/config.json.new > /dev/null && \
            sudo mv /usr/local/etc/xray/config.json.new /usr/local/etc/xray/config.json && \
            sudo systemctl reload-or-restart xray"#
        );
        let out = self.backend.exec_on_host(&add_cmd).await?;
        if !out.success() {
            return Err(AppError::Config(format!("add client: {}", out.stderr)));
        }
        Ok(ClientEntry { uuid, email })
    }

    pub async fn remove_client(&self, name: &str) -> Result<()> {
        let email = format!("{name}@vpn");
        let rm_cmd = format!(
            r#"sudo jq '.inbounds[0].settings.clients |= map(select(.email != "{email}"))' \
            /usr/local/etc/xray/config.json | sudo tee /usr/local/etc/xray/config.json.new > /dev/null && \
            sudo mv /usr/local/etc/xray/config.json.new /usr/local/etc/xray/config.json && \
            sudo systemctl reload-or-restart xray"#
        );
        let out = self.backend.exec_on_host(&rm_cmd).await?;
        if !out.success() {
            return Err(AppError::Config(format!("remove client: {}", out.stderr)));
        }
        Ok(())
    }

    pub async fn bridge_public_params(&self) -> Result<BridgePublicParams> {
        let out = self.backend.exec_on_host("sudo cat /usr/local/etc/xray/config.json").await?;
        let v: serde_json::Value = serde_json::from_str(&out.stdout)
            .map_err(|e| AppError::Config(format!("parse config: {e}")))?;
        let inbound = &v["inbounds"][0];
        let stream = &inbound["streamSettings"];
        Ok(BridgePublicParams {
            port: inbound["port"].as_u64().unwrap_or(443) as u16,
            sni: stream["realitySettings"]["serverNames"][0]
                .as_str().unwrap_or("").to_string(),
            short_id: stream["realitySettings"]["shortIds"][0]
                .as_str().unwrap_or("").to_string(),
            path: stream["xhttpSettings"]["path"]
                .as_str().unwrap_or("").to_string(),
            // PublicKey isn't in server.json — it derives from privateKey. We compute it via `xray x25519` reverse.
            // Simpler: store public key alongside. For now, read it from a sidecar file written at migrate time.
            public_key: Self::read_public_key(self.backend).await?,
        })
    }

    async fn read_public_key(backend: &dyn XrayBackend) -> Result<String> {
        // During migrate, we write /usr/local/etc/xray/reality-public-key to make bot self-sufficient.
        let out = backend.exec_on_host("sudo cat /usr/local/etc/xray/reality-public-key 2>/dev/null").await?;
        let key = out.stdout.trim().to_string();
        if key.is_empty() {
            return Err(AppError::Config("reality-public-key file missing — migrate-bridge should have created it".into()));
        }
        Ok(key)
    }
}

#[derive(Debug, Clone)]
pub struct BridgePublicParams {
    pub port: u16,
    pub sni: String,
    pub short_id: String,
    pub path: String,
    pub public_key: String,
}
```

- [ ] **Step 3: Update `migrate/bridge.rs` phase 2 to write `/usr/local/etc/xray/reality-public-key`**

After `write_xray_config`, add:

```rust
let pubkey_cmd = format!(
    "echo '{pub}' | sudo tee /usr/local/etc/xray/reality-public-key > /dev/null && sudo chmod 644 /usr/local/etc/xray/reality-public-key",
    pub = secrets.reality_public
);
new_backend.exec_on_host(&pubkey_cmd).await?;
```

- [ ] **Step 4: Build**

Run: `cargo build 2>&1 | tail -3`

- [ ] **Step 5: Commit**

```bash
git add src/native/ src/migrate/bridge.rs
git commit -m "feat(native): NativeXrayClient and NativeBackend; persist public key on bridge"
```

---

### Task 6.2: Bot `--bridge` switch + rewire `/users` handler

**Files:**
- Modify: `src/telegram.rs`
- Modify: `src/main.rs` (bot startup)

- [ ] **Step 1: Add branch in telegram.rs**

Find the section that creates `BotState` and manages commands. Where current code calls `XrayApiClient::new(&*state.backend)`, introduce a branch:

```rust
// pseudocode — exact location depends on existing structure:
// In the command handler for /users:

if state.config.bridge {
    let client = crate::native::client::NativeXrayClient::new(&*state.backend);
    let clients = client.list_clients().await?;
    let msg = format!("Users ({}):\n{}", clients.len(),
        clients.iter().map(|c| format!("• {}", c.email.trim_end_matches("@vpn")))
            .collect::<Vec<_>>().join("\n"));
    bot.send_message(chat_id, msg).await?;
} else {
    // existing Amnezia path
    let client = XrayApiClient::new(&*state.backend);
    // ... existing code ...
}
```

Wrap similar branches for `/add`, `/delete`, `/url`, `/qr`.

- [ ] **Step 2: Update bot startup in main.rs to choose backend**

In `cli_telegram_bot()`:

```rust
let backend: Box<dyn XrayBackend> = if cli.bridge {
    // NativeBackend: local or SSH
    if cli.local {
        Box::new(LocalBackend::new(String::new(), "localhost".into()))
        // NativeLocalBackend variant could replace this; for now LocalBackend with empty container works
        // because NativeXrayClient doesn't call exec_in_container.
    } else {
        connect_ssh_native(cli.ssh_host.as_deref().unwrap_or("")).await?
    }
} else {
    // existing Amnezia path
    connect_cli_backend(config, cli.local)
};
```

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -3`
Expected: success with possible deprecation/unused warnings — fix as needed.

- [ ] **Step 4: Commit**

```bash
git add src/telegram.rs src/main.rs
git commit -m "feat(bot): --bridge switch routes bot through NativeXrayClient"
```

---

### Task 6.3: /add returns URL + QR by default

**Files:**
- Modify: `src/telegram.rs`

- [ ] **Step 1: Update `/add` handler**

In the `/add` handler branch for `--bridge` mode:

```rust
if state.config.bridge {
    let client = crate::native::client::NativeXrayClient::new(&*state.backend);
    let entry = client.add_client(&name).await?;
    let params = client.bridge_public_params().await?;
    let url = crate::native::url::render_xhttp_url(&crate::native::url::XhttpUrlParams {
        uuid: entry.uuid,
        host: state.backend.hostname().to_string(),
        port: params.port,
        path: params.path.clone(),
        sni: params.sni.clone(),
        public_key: params.public_key.clone(),
        short_id: params.short_id.clone(),
        name: name.clone(),
    });
    // Send URL message
    bot.send_message(chat_id, format!("✅ Added `{name}`\n\n`{url}`"))
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;
    // Send QR as photo
    let png = crate::native::url::render_qr_png(&url)?;
    use teloxide::types::InputFile;
    bot.send_photo(chat_id, InputFile::memory(png)).await?;
    return Ok(());
}
```

- [ ] **Step 2: CLI `--add-user` also shows URL + QR ASCII**

In `src/main.rs` `cli_add_user()`, add a branch that prints ASCII QR after URL:

```rust
// After printing URL:
let ascii_qr = amnezia_xray_admin::native::url::render_qr_ascii(&url);
println!("\n{ascii_qr}");
```

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -3`

- [ ] **Step 4: Commit**

```bash
git add src/telegram.rs src/main.rs
git commit -m "feat(bot,cli): /add and --add-user return URL + QR by default"
```

---

## Phase 7 — Docs and smoke tests

### Task 7.1: CHANGELOG entry

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add section**

At top of `CHANGELOG.md`, under existing `## [Unreleased]`:

```markdown
## [Unreleased]

### Added
- `migrate-bridge --new-ssh <alias> --old-ssh <alias>` — provision fresh bridge VPS,
  regenerate keys, cut over users. Optional bot redeploy via `--telegram-token --admin-id`.
- `migrate-egress --new-ssh <alias> --old-ssh <alias> --bridge-ssh <alias>` — provision
  fresh egress VPS, Let's Encrypt cert, nginx self-steal, atomic bridge-outbound cutover.
  `--duckdns-token` for automated DNS update.
- `--bridge` bot mode: drives native xray on the new bridge (no Amnezia Docker).
- `/add` (bot) and `--add-user` (CLI) now return both the URL and a QR code by default.

### Changed
- Internal: new `src/native/` module for native-xray operations (Reality+XHTTP).
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: changelog entry for Epic B+C"
```

---

### Task 7.2: CLAUDE.md subcommand docs

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add section**

Append to the "CLI commands" block in `CLAUDE.md`:

```markdown
### VPS migration

```bash
# Migrate bridge to a fresh VPS (SSH alias 'yc-new' must exist in ~/.ssh/config)
amnezia-xray-admin --migrate-bridge --new-ssh yc-new --old-ssh yc-vm

# With bot redeploy
amnezia-xray-admin --migrate-bridge --new-ssh yc-new --old-ssh yc-vm \
    --telegram-token $BOT_TOKEN --admin-id 12345

# Migrate egress
amnezia-xray-admin --migrate-egress --new-ssh vps-new --old-ssh vps-vpn \
    --bridge-ssh yc-vm --duckdns-token $DUCKDNS_TOKEN
```

Both commands support `--dry-run` and `--yes`.
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document migrate-bridge and migrate-egress in CLAUDE.md"
```

---

### Task 7.3: Dry-run smoke test

- [ ] **Step 1: Build release**

Run: `cargo build --release`

- [ ] **Step 2: Dry-run migrate-bridge**

```bash
./target/release/amnezia-xray-admin --migrate-bridge --new-ssh yc-vm --old-ssh yc-vm --dry-run
```

Expected: prints plan, does not touch servers. Document output in PR description.

- [ ] **Step 3: `cargo test`**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 4: `cargo clippy`**

Run: `cargo clippy -- -D warnings 2>&1 | tail -10`
Expected: clean (or only pre-existing warnings).

- [ ] **Step 5: `cargo fmt --check`**

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 6: Final commit if anything changed for lints**

```bash
git add -u
git commit -m "chore: clippy + fmt fixes" || echo "nothing to commit"
```

---

## Phase 8 — Live acceptance

These are **manual** steps; operator runs on real infra.

### Task 8.1: Live bridge migration dry run

- [ ] Create a second Yandex Cloud VPS (yc-vm-test).
- [ ] Add SSH alias in `~/.ssh/config`.
- [ ] Run: `./target/release/amnezia-xray-admin --migrate-bridge --new-ssh yc-vm-test --old-ssh yc-vm` (without `--yes`, review each phase).
- [ ] Verify: listing shows clients on new host with new UUIDs.
- [ ] Verify: URLs from stdout connect successfully from a test Xray client.
- [ ] Verify: after cutover confirmation, old `yc-vm` has xray stopped.

### Task 8.2: Live egress migration

- [ ] Provision a test VPS (Hetzner free-tier or similar).
- [ ] Have DuckDNS token ready.
- [ ] Run: `--migrate-egress --new-ssh vps-new --old-ssh vps-vpn --bridge-ssh yc-vm --duckdns-token <T>`.
- [ ] Verify: DuckDNS resolves to new IP.
- [ ] Verify: Let's Encrypt cert issued.
- [ ] Verify: bridge xray restarted with new outbound (tail error.log).
- [ ] Verify: real client traffic through bridge → new egress (check public IP via curl ifconfig.me from phone on VPN).

### Task 8.3: Bot acceptance

- [ ] After bridge migration with `--telegram-token/--admin-id`, verify bot alive:
      `ssh yc-vm-test 'systemctl is-active amnezia-xray-bot'` → active.
- [ ] In Telegram: `/users`, `/add testuser`, `/url testuser`, `/qr testuser`, `/delete testuser`.
- [ ] `/add` should reply with URL string **plus** a QR photo (not just a button).
- [ ] CLI: `amnezia-xray-admin --bridge --add-user cli_test` prints URL and an ASCII QR in terminal.

---

## Self-Review Checklist

- [ ] All new types (`BridgeConfigInput`, `EgressConfigInput`, `ClientEntry`, `EgressOutbound`, `Secrets`, `XhttpUrlParams`, `BridgePublicParams`) appear in tasks that define them before being used.
- [ ] Every `render_*` / `parse_*` / `install::*` function has a unit test in the same phase where it's defined.
- [ ] Every subcommand dispatch in `main.rs` has matching CLI flags in `config.rs`.
- [ ] Bot `/add` and CLI `--add-user` both call `render_qr_ascii` / `render_qr_png` (one each).
- [ ] CHANGELOG and CLAUDE.md updated.
- [ ] No placeholders: every code block is runnable as-is (modulo imports which the engineer adds).
