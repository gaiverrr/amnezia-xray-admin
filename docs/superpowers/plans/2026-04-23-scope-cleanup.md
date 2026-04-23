# Epic D Implementation Plan — Scope Cleanup

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the TUI subsystem and all legacy Amnezia-Docker code paths. Merge `src/native/` into core modules. Release as v0.4.0.

**Architecture:** Mostly deletion + consolidation, not new features. Order matters: move files out of `src/native/` before deleting `src/xray/` legacy so imports don't break. Verify `cargo build` + `cargo test` after every file-level change.

**Tech Stack:** Rust 2021. No new crates; remove ratatui/crossterm family. Work in branch `refactor/scope-cleanup`.

**Spec:** `docs/superpowers/specs/2026-04-23-scope-cleanup-design.md`.

---

## Target file structure (post-refactor)

```
src/
├── main.rs              # CLI parsing + dispatch (shrinks)
├── config.rs            # Cli struct (slimmer), config.toml I/O
├── ssh.rs               # Unchanged
├── error.rs             # Unchanged
├── backend_trait.rs     # XrayBackend trait + LocalBackend + SshBackend (no docker wrap)
├── telegram.rs          # Telegram bot (single code path, no legacy branches)
├── xray/
│   ├── mod.rs
│   ├── client.rs        # XrayClient (was src/native/client.rs::NativeXrayClient)
│   ├── config_render.rs # Was src/native/config_render.rs
│   ├── url.rs           # Was src/native/url.rs
│   └── types.rs         # Trimmed: XrayUser, TrafficStats, ServerInfo only
├── migrate/
│   ├── mod.rs           # pub use install::*
│   └── install.rs       # Unchanged provisioning helpers (kept per spec)
tests/fixtures/          # Unchanged (bridge-config-sample, egress-config-sample)
```

**Deleted:**
- `src/ui/` (entire dir)
- `src/app.rs`
- `src/backend.rs` (helpers inlined where used)
- `src/native/` (entire dir after move)
- `src/xray/snapshot.rs`
- `src/xray/config.rs` (Amnezia `ensure_api_enabled` etc.)

---

## Conventions

- Work on branch `refactor/scope-cleanup`. Commit after each task, not within tasks.
- After each task: `cargo build 2>&1 | tail -3` must succeed; `cargo test --bin amnezia-xray-admin 2>&1 | tail -3` must pass.
- If a compile error requires a change that's not part of the task, make it inline but note in commit message.
- No TDD for deletion work — changes are structural. Instead: verify behaviour via existing test suite + `cargo run -- --list-users` smoke at end.

---

## Phase 0 — Branch + baseline

### Task 0.1: Branch and baseline

- [ ] **Step 1: Create branch**

```bash
git checkout main
git pull --ff-only origin main
git checkout -b refactor/scope-cleanup
```

- [ ] **Step 2: Record baseline**

```bash
find src -name '*.rs' | xargs wc -l | tail -1  # total LOC now
cargo test --bin amnezia-xray-admin 2>&1 | tail -1  # test count
```

Expected: ~16k LOC, 473 tests passing.

- [ ] **Step 3: Commit baseline note**

```bash
echo "# Epic D scope cleanup started $(date -I)" > .epic-d-baseline
git add .epic-d-baseline
git commit -m "chore: start Epic D scope cleanup"
```

---

## Phase 1 — Delete TUI subsystem

### Task 1.1: Delete `src/ui/` and `src/app.rs`

**Files:**
- Delete: `src/ui/` (whole directory)
- Delete: `src/app.rs`
- Modify: `src/main.rs` — remove `mod ui;`, `mod app;`, and the TUI launch fallthrough at the end of `main()`

- [ ] **Step 1: Delete files**

```bash
rm -rf src/ui/
rm src/app.rs
```

- [ ] **Step 2: Update `src/main.rs`**

Remove lines:
- `mod ui;` (if present)
- `mod app;`
- Anything inside `fn main()` after the last CLI-subcommand dispatch that launches `App::new(...).run(...)` or similar.

Replace the TUI fallthrough with printing help:

```rust
// If no CLI subcommand matched, print help and exit 1.
Cli::command().print_help().ok();
std::process::exit(1);
```

Add `use clap::CommandFactory;` at the top of `main.rs` (needed for `Cli::command()`).

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | tail -15
```

Expected: compile errors referencing undefined items from `backend.rs` (BackendMsg, spawn_*). That's fine — Task 1.2 cleans those up.

If errors reference `app::` or `ui::` specifically → delete those references in main.rs and retry. If they reference `telegram_setup`, `qr` (UI mod), etc. → same treatment.

- [ ] **Step 4: Commit (even if not yet building)**

```bash
git add -A
git commit -m "refactor: delete TUI (src/ui/, src/app.rs)

Build will fail until src/backend.rs helpers are inlined (Task 1.2).
Staged as separate commit for reviewability."
```

### Task 1.2: Delete `src/backend.rs`, inline the 3 still-referenced helpers

**Files:**
- Delete: `src/backend.rs`
- Modify: `src/main.rs` — inline `fetch_latest_xray_version` / `detect_bot_container` / anything else still called; remove `mod backend;`

- [ ] **Step 1: Identify what's still referenced**

```bash
grep -rn "backend::" src/ 2>&1 | grep -v "backend_trait::" | head
```

Expected: hits in `src/main.rs` for a few helpers. Probably:
- `backend::build_vless_url`
- `backend::fetch_latest_xray_version`
- `backend::detect_bot_container`

- [ ] **Step 2: Inline them**

For each helper still called, copy the function body from `src/backend.rs` into `src/main.rs` as a private `async fn` right above `main()`. Example shape:

```rust
async fn detect_bot_container(backend: &dyn XrayBackend) -> Option<String> {
    // ... body copied from src/backend.rs, un-pub'd ...
}
```

If `build_vless_url` constructs legacy Vision URLs (it does — Amnezia path), **do not** inline it. It will be replaced entirely in Task 3.x when Amnezia code goes. For now, `grep` every call-site of `backend::build_vless_url` in `src/main.rs` and delete those call-sites (they're in `cli_add_user` / `cli_user_url` legacy branches which get simplified in Phase 4).

- [ ] **Step 3: Delete `src/backend.rs` and the `mod backend;` line in `src/main.rs`**

```bash
rm src/backend.rs
```

- [ ] **Step 4: Build**

```bash
cargo build 2>&1 | tail -15
```

Expected: more errors (still), but scope shifts toward legacy Amnezia types.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: delete src/backend.rs, inline 2-3 helpers in main.rs

TUI task spawners and BackendMsg are gone. build_vless_url call-sites
are left failing and will be resolved in Phase 4 when CLI dispatches
switch to bridge-only flow."
```

### Task 1.3: Drop TUI deps from `Cargo.toml`

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock` (by `cargo build`)

- [ ] **Step 1: Remove from `[dependencies]`**

Delete lines matching:
- `ratatui`
- `crossterm`
- `tui-textarea` (if present)
- `arboard` (check — if only used by UI clipboard)

```bash
grep -nE '^(ratatui|crossterm|tui-textarea|arboard)' Cargo.toml
```

Use `Edit` tool on `Cargo.toml` to remove matching lines (one by one).

- [ ] **Step 2: Regen lockfile, verify reduction**

```bash
cargo build 2>&1 | tail -5
```

Build still fails elsewhere — but the TUI-deps should now be absent:

```bash
grep -cE '^name = "(ratatui|crossterm|tui-textarea)"' Cargo.lock
```

Expected: 0.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: drop ratatui/crossterm deps (TUI gone)"
```

---

## Phase 2 — Merge `src/native/` into core

### Task 2.1: Delete legacy `src/xray/client.rs` and move `src/native/client.rs` in its place

**Files:**
- Delete: `src/xray/client.rs` (legacy `XrayApiClient`)
- Move: `src/native/client.rs` → `src/xray/client.rs`
- Rename: `NativeXrayClient` → `XrayClient` inside that file and at all call-sites

- [ ] **Step 1: Delete legacy**

```bash
rm src/xray/client.rs
```

- [ ] **Step 2: Move + rename**

```bash
mv src/native/client.rs src/xray/client.rs
```

Edit `src/xray/client.rs`:
- Replace `NativeXrayClient` → `XrayClient` (struct decl + `impl` blocks).
- Update doc-comment header to reflect new role.
- Update `use crate::native::config_render` → `use crate::xray::config_render`.

- [ ] **Step 3: Update imports at call-sites**

```bash
grep -rn "NativeXrayClient\|native::client::NativeXrayClient\|crate::native::client" src/ 2>&1 | head
```

In `src/telegram.rs` and `src/main.rs`:
- Replace `NativeXrayClient::new(...)` → `XrayClient::new(...)`.
- Replace `crate::native::client::NativeXrayClient` → `crate::xray::client::XrayClient`.
- Replace `use crate::native::client::NativeXrayClient;` → `use crate::xray::client::XrayClient;`.

- [ ] **Step 4: Build**

```bash
cargo build 2>&1 | tail -10
```

Expected: reduced error count; remaining errors mostly in `src/xray/config.rs` and `src/xray/snapshot.rs` legacy references.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: replace src/xray/client.rs with NativeXrayClient (renamed XrayClient)"
```

### Task 2.2: Merge `src/native/backend.rs` into `src/backend_trait.rs`

**Files:**
- Modify: `src/backend_trait.rs` — replace `LocalBackend` + `SshBackend` bodies with the versions from `src/native/backend.rs::NativeLocalBackend` + `NativeSshBackend` (they are simpler, no docker-exec wrap)
- Delete: `src/native/backend.rs`

- [ ] **Step 1: Edit `src/backend_trait.rs`**

Replace existing `LocalBackend` impl (which does `docker_exec_args(cmd)` wrapping) with the `NativeLocalBackend` body:

```rust
pub struct LocalBackend {
    hostname: String,
}

impl LocalBackend {
    pub fn new(hostname: String) -> Self {
        Self { hostname }
    }

    async fn run_shell(&self, cmd: &str) -> Result<CommandOutput> {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(AppError::Io)?;
        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1) as u32,
        })
    }
}

#[async_trait]
impl XrayBackend for LocalBackend {
    async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput> {
        self.run_shell(cmd).await
    }
    async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput> {
        self.run_shell(cmd).await
    }
    fn container_name(&self) -> &str { "" }
    fn hostname(&self) -> &str { &self.hostname }
}
```

Same shape for `SshBackend` — its `exec_in_container` now passes straight through to `self.session.exec_command(cmd)` (no docker wrapping). Keep the existing SSH constructor.

The old `LocalBackend::docker_exec_args` and constructors that took a `container` argument: delete.

- [ ] **Step 2: Delete `src/native/backend.rs`**

```bash
rm src/native/backend.rs
```

- [ ] **Step 3: Update `src/native/mod.rs`**

Remove `pub mod backend;` line.

- [ ] **Step 4: Fix call-sites**

```bash
grep -rn "NativeLocalBackend\|NativeSshBackend" src/ 2>&1 | head
```

In `src/main.rs`, replace:
- `NativeLocalBackend::new(h)` → `LocalBackend::new(h)`
- `NativeSshBackend::new(session, h)` → `SshBackend::new(session, h)`

Also drop any `--container` argument-passing at these call-sites since the native backends don't take one.

- [ ] **Step 5: Build**

```bash
cargo build 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: merge NativeLocalBackend/NativeSshBackend into core

Drops docker-exec wrapping in LocalBackend and SshBackend. Both now
talk to native xray on the remote host."
```

### Task 2.3: Move `src/native/{config_render,url}.rs` → `src/xray/`

- [ ] **Step 1: Move files**

```bash
mv src/native/config_render.rs src/xray/config_render.rs
mv src/native/url.rs src/xray/url.rs
```

- [ ] **Step 2: Update import paths**

```bash
grep -rn "crate::native::\(config_render\|url\)\|use crate::native::config_render\|use crate::native::url" src/ 2>&1 | head
```

Replace `crate::native::config_render` → `crate::xray::config_render`, same for `url`.

In the `tests/fixtures/*` `include_str!` paths inside `src/xray/config_render.rs`: path relative to the file changed from `src/native/config_render.rs` → `src/xray/config_render.rs`. Both are two levels deep (`../../tests/fixtures/...`), so no change needed — but verify by running tests.

- [ ] **Step 3: Delete `src/native/`**

```bash
rm -rf src/native/
```

Remove `pub mod native;` from `src/main.rs`.

Add `pub mod config_render; pub mod url;` to `src/xray/mod.rs`.

- [ ] **Step 4: Build + test**

```bash
cargo build 2>&1 | tail -5
cargo test --bin amnezia-xray-admin 2>&1 | tail -3
```

If some tests still fail — they're probably snapshot tests that reference `native::config_render::tests::...`. Their module path changed to `xray::config_render::tests::...`, but test bodies are identical; they run by new path.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: move src/native/{config_render,url}.rs → src/xray/; delete src/native/"
```

---

## Phase 3 — Delete Amnezia legacy code

### Task 3.1: Delete `src/xray/snapshot.rs`

**Files:**
- Delete: `src/xray/snapshot.rs`
- Modify: `src/xray/mod.rs` — remove `pub mod snapshot;`
- Modify: any references in `src/main.rs`, `src/telegram.rs` to `xray::snapshot::*` — delete those call-sites

- [ ] **Step 1: Identify call-sites**

```bash
grep -rn "xray::snapshot\|snapshot::" src/ 2>&1 | head
```

Expected: hits in main.rs (`cli_backup`, `cli_restore`), telegram.rs (`/snapshot`, `/snapshots`, `/restore`, `/upgrade` handlers).

- [ ] **Step 2: Delete file**

```bash
rm src/xray/snapshot.rs
```

- [ ] **Step 3: Remove `pub mod snapshot;` from `src/xray/mod.rs`**

- [ ] **Step 4: Delete CLI commands in `src/main.rs`**

Remove blocks dispatching `cli.backup` / `cli.restore` / `cli.upgrade_xray` (if upgrade_xray lives in snapshot.rs — if it lives elsewhere, leave). Also delete the `cli_backup` / `cli_restore` fns.

- [ ] **Step 5: Delete Telegram bot snapshot/restore/upgrade handlers**

In `src/telegram.rs`:
- Remove the `Command::Snapshot`, `Command::Snapshots`, `Command::Restore`, `Command::Upgrade` variants from `Command` enum.
- Delete their handler match-arms and helper functions (`cmd_upgrade`, etc.).
- Delete `RESTORE_PREFIX`, `UPGRADE_CONFIRM_PREFIX` callback handlers.
- Delete `bridge_unsupported_msg()` — it's now unreachable because those commands are gone, not disabled.

- [ ] **Step 6: Build + test**

```bash
cargo build 2>&1 | tail -5
cargo test --bin amnezia-xray-admin 2>&1 | tail -3
```

Tests that exercised snapshot/restore/upgrade on the bot will fail compilation. Delete those tests outright.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: delete src/xray/snapshot.rs and all snapshot/restore/upgrade commands

/snapshot /snapshots /restore /upgrade bot commands are gone. CLI
--backup --restore --upgrade-xray flags stripped. Those workflows
live in .claude/skills/amnezia-ops/ now."
```

### Task 3.2: Delete legacy `src/xray/config.rs` (Amnezia `ensure_api_enabled` etc.)

**Files:**
- Delete: `src/xray/config.rs`
- Modify: any references in `src/main.rs`, `src/telegram.rs`

- [ ] **Step 1: Identify call-sites**

```bash
grep -rn "xray::config::\|ensure_api_enabled\|read_server_config\|read_clients_table\|SERVER_CONFIG_PATH\|CLIENTS_TABLE_PATH" src/ 2>&1 | head
```

- [ ] **Step 2: Delete file**

```bash
rm src/xray/config.rs
```

- [ ] **Step 3: Remove `pub mod config;` from `src/xray/mod.rs`**

- [ ] **Step 4: Delete call-sites**

Main culprit: `src/main.rs` probably has `cli_check_server` that calls `ensure_api_enabled`. Either:
- Delete `cli_check_server` and the `--check-server` flag.
- Or reimplement in bridge mode: `XrayClient::bridge_public_params()` returns everything you need to confirm xray is serving.

For v0.4.0 — delete `cli_check_server` and `--check-server` (it was legacy-only; bridge has `systemctl is-active xray` on the host).

- [ ] **Step 5: Build + test**

```bash
cargo build 2>&1 | tail -5
cargo test --bin amnezia-xray-admin 2>&1 | tail -3
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: delete src/xray/config.rs (Amnezia ensure_api_enabled and friends)"
```

### Task 3.3: Trim `src/xray/types.rs`

**Files:**
- Modify: `src/xray/types.rs`

- [ ] **Step 1: Identify what's still used**

```bash
grep -rn "xray::types\|crate::xray::types" src/ 2>&1 | head
```

Expected: `XrayUser`, `TrafficStats`, `ServerInfo` used in `telegram.rs::format_users_message` / `format_status_message`.

- [ ] **Step 2: Delete unused types**

Remove from `src/xray/types.rs`:
- `ServerConfig` (wrapper around `serde_json::Value` for Amnezia server.json) — nothing uses it now.
- `ClientsTable` — nothing uses it now.
- Helper methods on the above.

Keep only `XrayUser`, `TrafficStats`, `ServerInfo` plus any `impl Default` they need.

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add src/xray/types.rs
git commit -m "refactor: trim src/xray/types.rs (drop Amnezia ServerConfig/ClientsTable)"
```

---

## Phase 4 — Simplify CLI surface

### Task 4.1: Strip stale flags from `src/config.rs::Cli`

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Remove flags**

Remove these from `Cli` struct and any associated tests:
- `migrate_bridge`, `migrate_egress`, `new_ssh`, `old_ssh`, `bridge_ssh`, `duckdns_token`, `dry_run`, `yes`, `skip_old` (from Epic B, never wired beyond flags).
- `bridge` — no longer needed; the tool always runs in bridge mode.
- `ssh_host`, `container` — Amnezia-specific entry points.
- `deploy_bot`, `telegram_token` *only if* `telegram_token` was also used by `deploy-bot`; keep `telegram_token` otherwise (bot still needs it via env).
- `backup`, `restore`, `rename_user`, `check_server`, `upgrade_xray` (removed in Phase 3 already).

Also update the three `Cli { ... }` literal test instantiations in that file (`test_merge_cli_*`) — remove deleted fields.

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | tail -5
```

Expected: errors at dispatch call-sites in `src/main.rs` referencing removed fields. Task 4.2 fixes.

- [ ] **Step 3: Commit (tests likely still broken — OK)**

```bash
git add src/config.rs
git commit -m "refactor: strip migrate/bridge/amnezia-mode flags from Cli struct"
```

### Task 4.2: Simplify `src/main.rs` dispatch

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Delete dispatch arms for removed flags**

In the long `if cli.foo { … }` chain inside `main()`, remove:
- `if cli.migrate_bridge` / `migrate_egress` blocks.
- `if cli.check_server` block + its `cli_check_server` fn.
- `if cli.backup` / `restore` / `upgrade_xray` blocks + their fns.
- `if cli.rename_user` block.
- `if cli.deploy_bot` block + `cli_deploy_bot` fn.

- [ ] **Step 2: Simplify connect helpers**

Remove `connect_cli_backend(config, local)` dead code. The only connection helpers needed are:
- `connect_local()` — returns `LocalBackend::new(hostname)`.
- `connect_ssh(alias)` — returns `SshBackend::new(session, hostname)`.

Both construct simple native backends (no container arg). Inline directly, no more trait-object juggling.

- [ ] **Step 3: Build + test**

```bash
cargo build 2>&1 | tail -5
cargo test --bin amnezia-xray-admin 2>&1 | tail -3
```

Expected: clean build; some config tests may still need updates.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "refactor: simplify src/main.rs dispatch — bridge-only, one connect path"
```

---

## Phase 5 — Simplify `src/telegram.rs`

### Task 5.1: Remove `state.bridge` branches (always bridge mode)

**Files:**
- Modify: `src/telegram.rs`

- [ ] **Step 1: Remove the field and all branches**

In `BotState`: delete `pub bridge: bool`.

In each command handler, the current pattern is:

```rust
if state.bridge {
    let client = XrayClient::new(state.backend.as_ref());
    // ... bridge code ...
    return Ok((text, url));
}
// ... legacy Amnezia code ...
```

Replace with:

```rust
let client = XrayClient::new(state.backend.as_ref());
// ... bridge code ...
Ok((text, url))
```

Delete the legacy code bodies entirely. The "return Ok(...)" early-return is gone — the function body just flows through the bridge path.

Apply to every handler: `cmd_users`, `cmd_add`, `cmd_delete_prompt`, `cmd_delete_execute`, `cmd_url`, `cmd_qr`, `cmd_user_keyboard`, `cmd_status`. Also in the `Command::*` match-arms any residual `state.bridge` references.

- [ ] **Step 2: Remove `bridge_unsupported_msg`**

Grep:

```bash
grep -n "bridge_unsupported_msg\|bridge_unsupported" src/telegram.rs
```

Delete the function and all call-sites (they should all be already gone because the legacy-only commands `/snapshot /restore /upgrade` were deleted in Phase 3).

- [ ] **Step 3: Remove `XrayApiClient` imports**

```bash
grep -n "XrayApiClient" src/telegram.rs
```

All `XrayApiClient::new(...)` calls were in the legacy branches just deleted. Delete the `use crate::xray::client::XrayApiClient;` line (if still there).

- [ ] **Step 4: Update `BotState` construction in `src/main.rs::cli_telegram_bot`**

Remove the `bridge: bool` field from `BotState { ... }` constructor call.

- [ ] **Step 5: Build + test**

```bash
cargo build 2>&1 | tail -5
cargo test --bin amnezia-xray-admin 2>&1 | tail -3
```

- [ ] **Step 6: Commit**

```bash
git add src/telegram.rs src/main.rs
git commit -m "refactor(bot): drop bridge vs legacy branching; always use XrayClient"
```

### Task 5.2: Delete legacy-only bot tests

**Files:**
- Modify: `src/telegram.rs` tests module

- [ ] **Step 1: Identify tests that exercised the legacy branch**

```bash
cargo test --bin amnezia-xray-admin telegram 2>&1 | tail -30
```

Any test whose name includes `legacy`, `amnezia`, references `XrayApiClient`, or tests the `bridge_unsupported_msg` message — delete.

- [ ] **Step 2: Run tests**

```bash
cargo test --bin amnezia-xray-admin 2>&1 | tail -3
```

Expected: green.

- [ ] **Step 3: Commit**

```bash
git add src/telegram.rs
git commit -m "test(bot): prune tests exercising removed legacy code paths"
```

---

## Phase 6 — Quality gates

### Task 6.1: Full test + clippy + fmt

- [ ] **Step 1: Tests**

```bash
cargo test --bin amnezia-xray-admin 2>&1 | tail -3
```

Expected: all green. Count should be somewhere in the 300-400 range (down from 473 as legacy tests were removed).

- [ ] **Step 2: Clippy**

```bash
cargo clippy -- -D warnings 2>&1 | tail -10
```

Expected: clean. If there are dead-code warnings on items only used by removed code, delete those items.

- [ ] **Step 3: Fmt**

```bash
cargo fmt --check 2>&1 | tail -5 || cargo fmt
```

If `fmt --check` reports drift, run `cargo fmt` and commit:

```bash
git add -u
git commit -m "chore: cargo fmt after Phase 5"
```

### Task 6.2: Live smoke test

- [ ] **Step 1: CLI smoke**

```bash
cargo build --release
./target/release/amnezia-xray-admin --list-users
```

Expected: lists the 14 bridge users.

- [ ] **Step 2: CLI `--user-url` smoke**

```bash
./target/release/amnezia-xray-admin --user-url masha
```

Expected: vless URL printed + ASCII QR below.

---

## Phase 7 — Documentation

### Task 7.1: Rewrite `README.md`

**Files:**
- Overwrite: `README.md`

- [ ] **Step 1: New content**

Replace the whole file with (adjust personal wording to taste):

````markdown
# amnezia-xray-admin

Personal CLI + Telegram bot for running a double-hop Xray VPN
(RU bridge → foreign egress) for yourself and a few friends.

This is a hobby project, maintained by @gaiverrr for his own VPN
infrastructure. Not intended as a product; use at your own risk
if you want.

## What it does

- Manage Xray users on a native-systemd xray host (no Amnezia Docker).
- Add / remove users, generate vless URLs with QR codes.
- Run as a Telegram bot for day-to-day ops.
- Ship as a native binary via Homebrew.

## Install

```bash
brew install gaiverrr/tap/amnezia-xray-admin
```

## Use

```bash
# List users on the bridge
amnezia-xray-admin --list-users

# Add a user, print URL + ASCII QR
amnezia-xray-admin --add-user Alice

# Generate URL for existing user
amnezia-xray-admin --user-url Alice

# Run the Telegram bot locally on the bridge host
amnezia-xray-admin --telegram-bot --local --admin-id 123456 --host <bridge-ip>
```

See `CLAUDE.md` for details on current infrastructure and architecture.

Operational runbook for Claude Code users: `.claude/skills/amnezia-ops/`.

## License

MIT (per `LICENSE`).
````

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: rewrite README for current scope (hobby + personal VPN admin)"
```

### Task 7.2: Update `CLAUDE.md`

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Remove TUI and Amnezia Docker sections**

Delete these passages in `CLAUDE.md`:
- The Architecture section's "Three execution modes" → update to "Two modes: CLI, Telegram bot".
- The Architecture ASCII diagram that shows `SshBackend` + `LocalBackend` used by TUI and bot — simplify.
- Module Responsibilities entries for `backend.rs`, `app.rs`, `ui/`.
- Important Design Details entries for "No bind mounts", "clientsTable format", "Stats require level: 0" (Amnezia-specific) → keep only bridge-mode-relevant items.

- [ ] **Step 2: Add "Scope" preamble**

Near the top of `CLAUDE.md`, add:

```markdown
## Scope (as of 2026-04-23, v0.4.0)

This is **Yuriy's personal Xray VPN admin tool**, not a product.
Current functionality:

- CLI: `--list-users`, `--add-user`, `--delete-user`, `--user-url`,
  `--user-qr`, `--online-status`, `--server-info`.
- Telegram bot (`--telegram-bot`): `/users`, `/add`, `/delete`,
  `/url`, `/qr`, `/status`.
- Always talks to a native-systemd xray on the bridge host
  (`/usr/local/etc/xray/config.json`). No Amnezia-Docker support.
- Shipped via Homebrew formula in `gaiverrr/homebrew-tap`.

**Out of scope:** TUI (dropped in v0.4.0), Amnezia-Docker admin,
VPS migration subcommand (handled by `.claude/skills/amnezia-ops/`),
MTProxy management.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md for v0.4.0 scope (drop TUI, drop Amnezia Docker)"
```

### Task 7.3: Add `CHANGELOG.md` entry for v0.4.0

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add entry**

Insert at the top of the file, above existing `## [Unreleased]` or `## [0.3.0]`:

```markdown
## [Unreleased]

### Removed (breaking)
- Entire TUI subsystem (`src/ui/`, `src/app.rs`, `src/backend.rs`).
  `cargo run` without CLI flags now prints help and exits. If you
  used the interactive dashboard, use the CLI flags directly or
  the Telegram bot.
- Amnezia-Docker support: `XrayApiClient`, `ensure_api_enabled`,
  docker-exec wrapping in `LocalBackend`/`SshBackend`, `src/xray/snapshot.rs`.
- CLI flags: `--ssh-host`, `--container`, `--deploy-bot`,
  `--check-server`, `--backup`, `--restore`, `--upgrade-xray`,
  `--rename-user`, `--migrate-bridge`, `--migrate-egress` (and
  their arg family), `--bridge` (implicit now).
- Telegram bot commands: `/snapshot`, `/snapshots`, `/restore`,
  `/upgrade`, `/routes`, `/route`, `/unroute`.

### Changed
- `src/native/` contents absorbed into `src/xray/` and `src/backend_trait.rs`.
  `NativeXrayClient` → `XrayClient`. `NativeLocalBackend`/`NativeSshBackend`
  replace the docker-wrapping `LocalBackend`/`SshBackend`.

### Added
- Nothing new in this release — it's a scope-cleanup drop.

### Context
This release clarifies tool positioning as personal-use admin for
a double-hop Xray VPN. Size dropped from ~16k LOC to <10k LOC.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: CHANGELOG entry for v0.4.0 scope cleanup"
```

---

## Phase 8 — Release v0.4.0

### Task 8.1: Merge branch and release

- [ ] **Step 1: Final gate checks on branch**

```bash
cargo test --bin amnezia-xray-admin 2>&1 | tail -2
cargo clippy -- -D warnings 2>&1 | tail -2
cargo fmt --check && echo "fmt ✓"
```

- [ ] **Step 2: Merge to main**

```bash
git checkout main
git pull --ff-only origin main
git merge --no-ff refactor/scope-cleanup -m "refactor: Epic D — scope cleanup (drop TUI + Amnezia paths)"
```

- [ ] **Step 3: Run release script (minor bump 0.3.0 → 0.4.0)**

```bash
yes y | ./scripts/release.sh minor 2>&1 | tail -30
```

Wait for CI, GitHub release, Homebrew formula update.

- [ ] **Step 4: Verify release artifacts**

```bash
gh release view v0.4.0
git ls-remote --tags origin | grep v0.4.0
```

- [ ] **Step 5: Final cleanup**

```bash
git branch -d refactor/scope-cleanup
rm .epic-d-baseline   # delete the baseline marker from Task 0.1
git add -A
git commit -m "chore: remove epic-d baseline marker" --allow-empty
git push origin main
```

---

## Self-Review Checklist

- [ ] **Spec coverage:** every item in the spec's "Scope cuts" section has a deletion task (1.1, 1.2, 3.1, 3.2, 3.3, plus Phase 4 CLI stripping). Every item in "Keep" is addressed by NOT being deleted.
- [ ] **Phase ordering:** `src/native/client.rs` moves to `src/xray/client.rs` (Task 2.1) BEFORE `src/xray/config.rs` delete (Task 3.2), so legacy Amnezia types' last references are severed cleanly.
- [ ] **Ambiguity:** every `grep` call is concrete, file paths are exact. "Keep if still referenced" turns into the 2.x/3.x deletion tasks. No "TBD" items remain.
- [ ] **Types consistency:** `NativeXrayClient` → `XrayClient` renames are specified in Task 2.1 Step 2; call-site updates in Task 2.1 Step 3. `NativeLocalBackend` → `LocalBackend` specified in Task 2.2.
- [ ] **Tests:** Tasks 5.2 and 6.1 address test fallout. Task 6.2 adds live smoke. Expected final count: 300-400 (varies by how many legacy tests were coupled).
