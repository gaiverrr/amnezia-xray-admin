# Scope cleanup: drop TUI + legacy Amnezia-Docker paths

- **Status**: design approved, ready for implementation plan
- **Date**: 2026-04-23
- **Driver**: tool positioning clarified to hobby + personal infrastructure (not a product). Current codebase carries ~40% dead/legacy surface that no longer matches how the tool is actually used.

## Summary

Prune the codebase so it honestly reflects current usage. The tool is now a personal admin for Yuriy's double-hop Xray VPN (bridge + egress), driven through the Telegram bot day-to-day and CLI for one-shot ops. Remove:

1. **TUI subsystem** (~2500 LOC in `src/ui/`, `src/app.rs`, `src/backend.rs` task spawners).
2. **Legacy Amnezia-Docker code** (`XrayApiClient`, `clientsTable` handling, `ensure_api_enabled`, `LocalBackend`/`SshBackend` docker-exec wrapping, `src/xray/snapshot.rs` backup/restore).
3. **Stale CLI flags** (`--migrate-bridge`, `--migrate-egress` and their arg family — migration happens via `.claude/skills/amnezia-ops/`, not the binary).

Release result as **v0.4.0** with a clear breaking-change entry.

## Positioning (what the tool IS now)

- Personal admin for Yuriy's double-hop Xray VPN (`yc-vm` bridge → `vps-vpn` egress).
- Maintained as a hobby project, not a product — no attempt to attract external users.
- Two interfaces: **CLI** for one-shot ops and scripting, **Telegram bot** for day-to-day user management.
- Shipped via Homebrew formula in `gaiverrr/homebrew-tap`. No Docker image, no crates.io publish.

README and CLAUDE.md will state this plainly.

## Scope cuts (files to delete)

### TUI

- `src/ui/` — entire directory: `setup.rs`, `dashboard.rs`, `user_detail.rs`, `add_user.rs`, `qr.rs`, `telegram_setup.rs`, `theme.rs`, `mod.rs`. ~2000 LOC.
- `src/app.rs` — ratatui event loop + screen state machine. ~1400 LOC.
- `src/backend.rs` — async task spawners for TUI (`spawn_fetch_dashboard`, etc.) + `BackendMsg` enum. ~760 LOC. A few helpers here (`build_vless_url`, `fetch_latest_xray_version`, `detect_bot_container`) are still referenced outside TUI — **inline them at their call site** in `src/main.rs`. Delete the file.
- `src/main.rs` fallthrough that launches TUI when no CLI flag matches → replace with "print help" (or use `clap`'s default behaviour when no subcommand selected).

### `Cargo.toml` deps to remove

- `ratatui`
- `crossterm`
- `tui-textarea` (if present)
- `arboard` if only used by TUI (check usage)

### Legacy Amnezia-Docker code

- `src/xray/client.rs` — `XrayApiClient` type and all its methods (list/add/remove/rename, stats, online, backup/restore, `generate_vless_url` with `flow=xtls-rprx-vision`). Keep only `VlessUrlParams` struct if it's still used by CLI in non-bridge mode — **actually we drop non-bridge mode too (see below), so delete the whole file**.
- `src/xray/config.rs` — `ensure_api_enabled`, `backup_config`, `read_server_config`, `read_clients_table`, path constants for `/opt/amnezia/xray/…`. Delete entirely.
- `src/xray/snapshot.rs` — backup/restore for Amnezia's config files inside Docker. Delete.
- `src/xray/types.rs` — keep `XrayUser`, `TrafficStats`, `ServerInfo` if still referenced by bridge code. Remove `ServerConfig` (Amnezia wrapper) and `ClientsTable`.
- `src/backend_trait.rs::LocalBackend` and `SshBackend` — remove docker-exec wrapping logic. After this removal, `LocalBackend` and `SshBackend` become thin native-shell backends. Consolidate with `src/native/backend.rs::NativeLocalBackend`/`NativeSshBackend` — effectively rename `Native…` to just `LocalBackend`/`SshBackend` in `src/backend_trait.rs` and delete `src/native/backend.rs`.
- `shell_quote` utility (used for Amnezia email quoting) — delete; new code uses `validate_name`.

### CLI flags to remove

From `src/config.rs::Cli`:

- `--ssh-host` / `--container` — Amnezia-specific entry points; replaced by SSH alias + `--bridge`.
- `--migrate-bridge`, `--migrate-egress`, `--new-ssh`, `--old-ssh`, `--bridge-ssh`, `--duckdns-token`, `--dry-run`, `--skip-old` — reserved for a migration subcommand that we're not implementing.
- `--deploy-bot` — docker-image-wrapped bot deploy, no longer relevant; bot is deployed manually via scp + systemd unit on the bridge.
- Remove `--setup` (the interactive wizard) — the skill `.claude/skills/amnezia-ops/` covers bridge setup.

Keep (all default to bridge-mode since that's the only mode now):

- `--list-users`, `--add-user`, `--delete-user`, `--user-url`, `--user-qr`, `--online-status`, `--server-info`, `--check-server`.
- `--rename-user` — **remove for v0.4.0** (no bridge-mode implementation; file as follow-up task if a real need appears — renaming just changes `email` in `clients[]`, trivial to implement later).
- `--backup`, `--restore` — **remove for v0.4.0**. `ssh yc-vm 'sudo cat /usr/local/etc/xray/config.json > local-backup-$(date).json'` covers the use case trivially; no need for a subcommand.
- `--telegram-bot` + bot-mode flags (`--admin-id`, `--host`, `--telegram-token`).
- `--upgrade-xray` — runs `apt upgrade xray` on remote. Still useful.
- `--local`.

The `--bridge` flag becomes redundant (implicit — there is no other mode). Remove the flag. Bot and CLI always speak to native xray.

### Native refactor

- Rename `src/native/backend.rs::NativeLocalBackend` → move into `src/backend_trait.rs::LocalBackend` (merge them). Same for `NativeSshBackend` → `SshBackend`.
- Delete `src/native/backend.rs`.
- Rename `src/native/client.rs::NativeXrayClient` → `XrayClient` and move to `src/xray/client.rs` (replacing the deleted legacy one).
- The `src/native/` module effectively absorbs into `src/xray/` + `src/backend_trait.rs` since there's no longer a "native vs Amnezia" distinction.

## Keep (unchanged)

- `src/ssh.rs` — SSH session.
- `src/error.rs`.
- `src/telegram.rs` — Telegram bot (entry point for most users). Still the biggest single file; its internal split is tracked separately (`amnezia-xray-admin-k6o`).
- `src/migrate/install.rs` — unit-tested provisioning helpers; they might be invoked by skill-driven workflows in the future. Rest of `src/migrate/` (the empty bridge.rs/egress.rs stubs) — delete.
- `tests/fixtures/bridge-config-sample.json`, `tests/fixtures/egress-config-sample.json`.
- `.claude/skills/amnezia-ops/`.

## Docs

- **README.md** — rewrite. New intro: "Personal CLI + Telegram bot for running a double-hop Xray VPN (RU bridge → foreign egress) for yourself and a few friends." Install via Homebrew. Quick start: add SSH aliases, `brew install gaiverrr/tap/amnezia-xray-admin`, `amnezia-xray-admin --list-users`. Mention the `.claude/skills/amnezia-ops/` runbook for Claude Code users.
- **CLAUDE.md** — remove TUI section from Architecture and Module Responsibilities. Remove Amnezia-Docker design notes (`No bind mounts`, `clientsTable format`, `Stats require level: 0`). Add a "not in scope" section listing what the tool no longer does.
- **CHANGELOG.md** — `## [0.4.0]` entry: Removed TUI, Removed Amnezia legacy paths, Removed migration subcommand flags. Merged `Native*` into core modules.

## Shipping

- Homebrew formula at `gaiverrr/homebrew-tap` — keep. `release.sh` updates it automatically.
- GitHub Actions `release.yml` — still builds binaries for 4 platforms. Remains unchanged.
- No Docker image. No crates.io publish. No deb package.

## Implementation phases

1. Create branch `refactor/scope-cleanup`.
2. Delete `src/ui/`, `src/app.rs`; fix `main.rs` to print help on no flags.
3. Delete `src/backend.rs` (except `build_vless_url` if still used — move to `src/main.rs` or ops util).
4. Remove `ratatui`/`crossterm`/etc from `Cargo.toml`.
5. Delete `src/xray/client.rs`, `src/xray/config.rs` (`enable_api`), `src/xray/snapshot.rs`, stale parts of `src/xray/types.rs`.
6. Merge `src/native/backend.rs` → `src/backend_trait.rs` (rename types, unify).
7. Move `src/native/client.rs` → `src/xray/client.rs`. Rename `NativeXrayClient` → `XrayClient`.
8. Delete `src/native/` entirely (module absorbed).
9. Strip CLI flags in `src/config.rs`. Update `src/main.rs` dispatch accordingly.
10. Update `src/telegram.rs` — bot branches on `state.bridge` go away (always bridge mode). Simplify to single code path.
11. Re-run test suite; fix/update broken tests.
12. Rewrite README.
13. Update CLAUDE.md.
14. Update CHANGELOG.
15. Run `cargo test && cargo clippy -- -D warnings && cargo fmt --check`.
16. Merge to main, release v0.4.0 via `release.sh minor` (bumps 0.3.0 → 0.4.0).

## Acceptance

- `cargo build --release` builds; `src/` <10k LOC (from ~16k).
- `cargo test` passes; `cargo clippy -- -D warnings` clean; `cargo fmt --check` clean.
- `cargo run -- --list-users` works end-to-end against live bridge.
- `cargo run -- --telegram-bot --local --admin-id 775260 --host 81.26.189.136` runs the bot; `/users`, `/add`, `/delete`, `/url`, `/qr` all work.
- README first paragraph honestly describes current scope (no Amnezia-Docker as primary).
- CLAUDE.md has no references to `/opt/amnezia/xray/`, `clientsTable`, `ensure_api_enabled`, TUI, setup wizard.
- `gh release view v0.4.0` shows the new release with all 4 platform binaries.
- `brew upgrade amnezia-xray-admin` works from the tap.

## Non-goals (YAGNI, for this spec)

- Workspace split into multiple crates (filed separately — see "Future tasks").
- `telegram.rs` internal module split (already filed: `amnezia-xray-admin-k6o`).
- Bot `/users` stats display (already filed: `amnezia-xray-admin-oar`).
- MTProxy-through-bridge forwarder (already filed: `amnezia-xray-admin-7pg`).
- Repo rename — low value for hobby/personal, high cost on Homebrew/SEO continuity.
- Interactive setup wizard replacement — the skill covers it.

## Future tasks (filed as beads)

- `amnezia-xray-admin-k6o` — split `src/telegram.rs` into submodules (now with smaller file after this refactor, more reasonable).
- `amnezia-xray-admin-oar` — bot stats on bridge.
- `amnezia-xray-admin-7pg` — MTProxy through bridge.
- **NEW (to file): workspace split.** Break the binary into `core` (lib) + `cli` (bin) + `bot` (bin) crates. Enables independent shipping. Deferred — big effort, low near-term value.
