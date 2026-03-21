# Prepare v0.1.0 release for publication

## Overview
Prepare the app for public release via Homebrew tap and GitHub Releases. Users install with:
```
brew tap gaiverrr/tap
brew install amnezia-xray-admin
```

**What this plan covers:**
- Fix Cargo.toml metadata (repository URL, authors, keywords)
- Write proper README with installation, usage, screenshots
- Create CHANGELOG.md
- Set up GitHub Actions CI (test + clippy + release builds)
- Create Homebrew tap repo with formula
- Tag and publish v0.1.0

**What this plan does NOT cover:**
- Code refactoring (current code is fine for v0.1.0)
- crates.io publishing (not needed for a CLI binary)
- Cross-compilation for Windows (Linux + macOS only)

## Context
- GitHub user: `gaiverrr`
- Repo: `github.com/gaiverrr/amnezia-xray-admin`
- Homebrew tap: `github.com/gaiverrr/homebrew-tap`
- Current version: 0.1.0 in Cargo.toml
- License: MIT (already in place)
- README.md exists but needs polish
- No CI, no CHANGELOG, no git tags
- ralphex is currently executing `fix-cli-commands.md` plan (must finish first)

## Development Approach
- **Testing approach**: Regular (verify each step works)
- Complete each task fully before moving to the next
- **CRITICAL: all tests must pass before starting next task**
- **CRITICAL: update this plan file when scope changes**
- This plan should run AFTER the `fix-cli-commands` plan completes

## Progress Tracking
- Mark completed items with `[x]` immediately when done
- Add newly discovered tasks with ➕ prefix
- Document issues/blockers with ⚠️ prefix

## Implementation Steps

### Task 1: Fix Cargo.toml metadata
- [x] update `repository` to `https://github.com/gaiverrr/amnezia-xray-admin`
- [x] add `authors = ["gaiverrr"]`
- [x] add `homepage = "https://github.com/gaiverrr/amnezia-xray-admin"`
- [x] add `keywords = ["vpn", "xray", "amnezia", "tui", "vless"]`
- [x] add `categories = ["command-line-utilities", "network-programming"]`
- [x] add `readme = "README.md"`
- [x] verify `cargo build` still works
- [x] run tests — must pass before next task

### Task 2: Write README.md
- [x] rewrite README with sections: Overview, Features, Install, Quick Start, CLI Commands, Configuration, Screenshots, Contributing, License
- [x] installation section: Homebrew (primary), cargo install (secondary), from source
- [x] CLI commands section: document all `--list-users`, `--user-url`, `--user-qr`, `--online-status`, `--server-info`, `--check-server`
- [x] add TUI keybindings table
- [x] add screenshot placeholder (or real screenshot if available)
- [x] run tests — must pass before next task

### Task 3: Create CHANGELOG.md
- [x] create CHANGELOG.md following Keep a Changelog format
- [x] document v0.1.0 features: TUI dashboard, CLI commands, SSH connection, xray API integration, QR code generation, first-run setup wizard
- [x] run tests — must pass before next task

### Task 4: Set up GitHub Actions CI
- [x] create `.github/workflows/ci.yml`: test + clippy + fmt check on push/PR
- [x] matrix: ubuntu-latest + macos-latest
- [x] cache cargo registry and target dir
- [x] run tests — must pass before next task

### Task 5: Set up GitHub Actions release workflow
- [x] create `.github/workflows/release.yml`: triggered on tag push `v*`
- [x] build release binaries: linux-x86_64, linux-aarch64, macos-x86_64, macos-aarch64
- [x] use `cross` for Linux cross-compilation (or `cargo build --target`)
- [x] create GitHub Release with binaries attached
- [x] generate SHA256 checksums for each binary
- [x] run tests — must pass before next task

### Task 6: Create Homebrew tap
- [x] create `gaiverrr/homebrew-tap` repo on GitHub (via `gh repo create`)
- [x] create `Formula/amnezia-xray-admin.rb` Homebrew formula
- [x] formula downloads release tarball, builds from source with `cargo build --release`
- [x] add `test do` block that runs `amnezia-xray-admin --help`
- [x] verify formula syntax: `brew audit --strict Formula/amnezia-xray-admin.rb` (sha256 PLACEHOLDER errors expected until release)

### Task 7: Merge to main and create GitHub repo
- [x] create GitHub repo: `gh repo create gaiverrr/amnezia-xray-admin --public --source=. --push`
- [x] merge feature branch to main: `git checkout main && git merge amnezia-xray-admin-mvp.md`
- [x] push main to GitHub
- [x] verify CI passes on GitHub

### Task 8: Tag and release v0.1.0
- [ ] create git tag: `git tag -a v0.1.0 -m "Initial release"`
- [ ] push tag: `git push origin v0.1.0`
- [ ] verify release workflow creates GitHub Release with binaries
- [ ] update Homebrew formula with release URL and SHA256
- [ ] verify: `brew tap gaiverrr/tap && brew install amnezia-xray-admin`

### Task 9: Verify release
- [ ] verify `brew install` works on clean system
- [ ] verify `amnezia-xray-admin --help` shows version
- [ ] verify `amnezia-xray-admin --list-users` works
- [ ] verify GitHub Release page has all binaries
- [ ] run full test suite + clippy

### Task 10: [Final] Update documentation
- [ ] update CLAUDE.md with release process
- [ ] add "How to release" section to README or CONTRIBUTING.md

## Technical Details

**GitHub Actions CI** (`ci.yml`):
```yaml
name: CI
on: [push, pull_request]
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test
      - run: cargo clippy -- -D warnings
      - run: cargo fmt --check
```

**Release workflow** (`release.yml`):
```yaml
name: Release
on:
  push:
    tags: ['v*']
jobs:
  build:
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-latest
          - target: x86_64-apple-darwin
            os: macos-latest
          - target: aarch64-apple-darwin
            os: macos-latest
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - run: cargo build --release --target ${{ matrix.target }}
      - uses: softprops/action-gh-release@v2
        with:
          files: target/${{ matrix.target }}/release/amnezia-xray-admin
```

**Homebrew formula** (`Formula/amnezia-xray-admin.rb`):
```ruby
class AmneziaXrayAdmin < Formula
  desc "TUI dashboard for managing Amnezia VPN's Xray server"
  homepage "https://github.com/gaiverrr/amnezia-xray-admin"
  url "https://github.com/gaiverrr/amnezia-xray-admin/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "PLACEHOLDER"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "amnezia-xray-admin", shell_output("#{bin}/amnezia-xray-admin --help")
  end
end
```

## Post-Completion
- Announce release (social media, Reddit r/selfhosted, etc.)
- Consider submitting to homebrew-core after 30+ GitHub stars
- Consider crates.io publish if demand exists
- Set up Dependabot for dependency updates
