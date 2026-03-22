# Telegram bot UX: inline buttons + secure admin

## Overview
Two improvements to the Telegram bot:
1. **Inline buttons**: `/url`, `/qr`, `/delete` without argument show user list as inline keyboard buttons
2. **Secure admin**: admin ID set at deploy time via `--admin-id`, no more "first /start = admin" vulnerability

## Context
- Telegram bot code: `src/telegram.rs`
- Deploy code: `src/main.rs` (`cli_deploy_bot`), `src/backend.rs` (`deploy_bot`)
- Config: `src/config.rs` (Cli struct, Config struct)
- Current admin logic: first `/start` sender becomes admin, chat ID saved to file
- Bot uses `teloxide` crate with command handlers

## Development Approach
- **Testing approach**: CLI-first
- **CRITICAL: every task MUST include new/updated tests**
- **CRITICAL: all tests must pass before starting next task**
- Run `cargo test` after each change

## Progress Tracking
- Mark completed items with `[x]` immediately when done

## Implementation Steps

### Task 1: Replace auto-admin with --admin-id
- [x] add `--admin-id <ID>` arg to `Cli` struct in `src/config.rs` (i64 type for Telegram chat ID)
- [x] add `admin_id` field to `Config` struct (optional, saved to config.toml)
- [x] update `cli_deploy_bot()`: require `--admin-id` or prompt interactively. Pass admin ID to Docker container as env var `ADMIN_ID`
- [x] update `cli_telegram_bot()`: require admin_id from config/CLI/env. Fail with clear error if not set: "Admin ID required. Use --admin-id <your_telegram_id> or set ADMIN_ID env var"
- [x] update bot startup in `src/telegram.rs`: read admin ID from config instead of first-/start detection
- [x] remove first-/start-becomes-admin logic
- [x] update `/start` command: if sender == admin → "Welcome, admin!"; else → "Access denied. Contact the server administrator."
- [x] update all test Cli/Config struct instances with new field
- [x] write tests for admin ID validation
- [x] run tests — must pass before next task

### Task 2: Inline buttons for /url without argument
- [ ] modify `/url` handler in `src/telegram.rs`:
  - with argument: existing behavior (return URL for named user)
  - without argument: fetch user list, send inline keyboard with user names as buttons
- [ ] add callback query handler for "url:{user_name}" callback data
- [ ] when button pressed: generate URL, send as reply
- [ ] write tests for callback data parsing
- [ ] run tests — must pass before next task

### Task 3: Inline buttons for /qr without argument
- [ ] modify `/qr` handler:
  - without argument: show user list as inline keyboard
  - callback data: "qr:{user_name}"
- [ ] when button pressed: generate QR PNG, send as photo
- [ ] reuse inline keyboard building from Task 2 (extract helper function `build_user_keyboard(users, prefix)`)
- [ ] write tests for keyboard building
- [ ] run tests — must pass before next task

### Task 4: Inline buttons for /delete without argument
- [ ] modify `/delete` handler:
  - without argument: show user list as inline keyboard
  - callback data: "delete:{user_name}"
- [ ] when button pressed: show confirmation inline keyboard ("Yes, delete {name}" / "Cancel")
  - callback data: "confirm_delete:{user_name}" / "cancel_delete"
- [ ] on confirm: delete user, send success message
- [ ] on cancel: send "Cancelled" message
- [ ] write tests for delete confirmation flow
- [ ] run tests — must pass before next task

### Task 5: /add without argument prompts for name
- [ ] modify `/add` handler:
  - with argument: existing behavior
  - without argument: reply "Send me the user name:" and set conversation state to awaiting_name
- [ ] add simple state machine: when next text message arrives from admin → treat as user name, call add_user
- [ ] alternative simpler approach: just reply "Usage: /add <name>" with example
- [ ] write tests for handler behavior
- [ ] run tests — must pass before next task

### Task 6: Update deploy flow with admin-id
- [ ] update `deploy_bot()` in `src/backend.rs`: pass ADMIN_ID env var to Docker container
- [ ] update docker-compose generation (if exists) with ADMIN_ID
- [ ] update TUI Telegram setup screen: add admin ID field (or auto-detect hint)
- [ ] add hint in setup: "To find your Telegram ID: send /start to @userinfobot"
- [ ] write tests for deploy command generation with admin_id
- [ ] run tests — must pass before next task

### Task 7: Verify acceptance criteria
- [ ] verify /url without arg shows inline buttons
- [ ] verify /qr without arg shows inline buttons
- [ ] verify /delete without arg shows inline buttons + confirmation
- [ ] verify /add without arg shows usage hint
- [ ] verify --admin-id is required for --deploy-bot
- [ ] verify bot rejects non-admin users
- [ ] run full test suite: `cargo test`
- [ ] run linter: `cargo clippy`

### Task 8: [Final] Update documentation
- [ ] update CLAUDE.md with Telegram bot changes
- [ ] update README Telegram section with --admin-id
- [ ] update CHANGELOG.md

## Technical Details

**Inline keyboard helper:**
```rust
fn build_user_keyboard(users: &[XrayUser], callback_prefix: &str) -> InlineKeyboardMarkup {
    let buttons: Vec<Vec<InlineKeyboardButton>> = users
        .iter()
        .filter(|u| !u.name.is_empty())
        .map(|u| vec![InlineKeyboardButton::callback(
            &u.name,
            format!("{}:{}", callback_prefix, u.name),
        )])
        .collect();
    InlineKeyboardMarkup::new(buttons)
}
```

**Callback data format:** `"action:user_name"` — e.g. `"url:Alexander"`, `"qr:Tima"`, `"delete:Kostya"`, `"confirm_delete:Kostya"`

**Admin ID flow at deploy:**
```
$ cargo run -- --deploy-bot --telegram-token <TOKEN> --admin-id 123456789
```
Or interactively:
```
$ cargo run -- --deploy-bot --telegram-token <TOKEN>
Enter your Telegram ID (send /start to @userinfobot to find it): 123456789
```

## Post-Completion
- Manual testing: send commands to real bot, verify buttons work
- Test with non-admin user: verify access denied
