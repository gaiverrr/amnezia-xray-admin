#!/usr/bin/env bash
set -euo pipefail

# Release script for amnezia-xray-admin
# Usage: ./scripts/release.sh [patch|minor|major]
# Default: patch (0.1.8 → 0.1.9)

BUMP_TYPE="${1:-patch}"
REPO="gaiverrr/amnezia-xray-admin"
TAP_REPO="gaiverrr/homebrew-tap"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

step() { echo -e "${CYAN}▶ $1${NC}"; }
ok()   { echo -e "${GREEN}✓ $1${NC}"; }
warn() { echo -e "${YELLOW}⚠ $1${NC}"; }
fail() { echo -e "${RED}✗ $1${NC}"; exit 1; }

# ── Preflight checks ──────────────────────────────────────────────
step "Preflight checks..."

command -v cargo >/dev/null || fail "cargo not found"
command -v gh >/dev/null    || fail "gh CLI not found"
command -v jq >/dev/null    || fail "jq not found"

[[ "$(git branch --show-current)" == "main" ]] || fail "Not on main branch"
[[ -z "$(git status --porcelain)" ]]           || fail "Working tree not clean. Commit or stash changes first."

ok "On main, clean tree"

# ── Get current version ───────────────────────────────────────────
CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"

case "$BUMP_TYPE" in
    patch) PATCH=$((PATCH + 1)) ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
    *)     fail "Unknown bump type: $BUMP_TYPE (use patch, minor, or major)" ;;
esac

NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
TAG="v${NEW_VERSION}"

echo -e "  Current: ${YELLOW}${CURRENT}${NC}"
echo -e "  New:     ${GREEN}${NEW_VERSION}${NC} (${BUMP_TYPE})"
echo ""
read -p "Continue? [Y/n] " -n 1 -r
echo
[[ $REPLY =~ ^[Yy]?$ ]] || exit 0

# ── Run tests ─────────────────────────────────────────────────────
step "Running tests..."
cargo test --quiet 2>&1 | tail -1
cargo clippy --quiet -- -D warnings 2>&1
cargo fmt --check 2>&1
ok "Tests, clippy, fmt all pass"

# ── Bump version in Cargo.toml ────────────────────────────────────
step "Bumping version to ${NEW_VERSION}..."
sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW_VERSION}\"/" Cargo.toml
# Verify
grep "version = \"${NEW_VERSION}\"" Cargo.toml >/dev/null || fail "Version bump failed"
ok "Cargo.toml updated"

# ── Update CHANGELOG ──────────────────────────────────────────────
step "Updating CHANGELOG.md..."
TODAY=$(date +%Y-%m-%d)

# Check if there's content in [Unreleased]
if grep -qA1 '## \[Unreleased\]' CHANGELOG.md | grep -q '^$'; then
    warn "No unreleased changes in CHANGELOG. Add them now?"
    read -p "Open editor? [Y/n] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]?$ ]]; then
        ${EDITOR:-vim} CHANGELOG.md
    fi
fi

# Insert new version section after [Unreleased]
sed -i '' "s/## \[Unreleased\]/## [Unreleased]\n\n## [${NEW_VERSION}] - ${TODAY}/" CHANGELOG.md

# Update comparison links at bottom
if grep -q "\[Unreleased\]:" CHANGELOG.md; then
    sed -i '' "s|\[Unreleased\]:.*|[Unreleased]: https://github.com/${REPO}/compare/${TAG}...HEAD|" CHANGELOG.md
    # Add new version link before the [Unreleased] link... or after previous version
    PREV_TAG="v${CURRENT}"
    if ! grep -q "\[${NEW_VERSION}\]:" CHANGELOG.md; then
        sed -i '' "/\[Unreleased\]:/a\\
[${NEW_VERSION}]: https://github.com/${REPO}/compare/${PREV_TAG}...${TAG}" CHANGELOG.md
    fi
fi

ok "CHANGELOG.md updated for ${NEW_VERSION}"

# ── Commit ────────────────────────────────────────────────────────
step "Committing..."
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: release ${TAG}"
ok "Committed"

# ── Tag ───────────────────────────────────────────────────────────
step "Tagging ${TAG}..."
git tag -a "${TAG}" -m "${TAG}"
ok "Tagged"

# ── Push ──────────────────────────────────────────────────────────
step "Pushing to origin..."
git push origin main --tags
ok "Pushed main + ${TAG}"

# ── Wait for CI ───────────────────────────────────────────────────
step "Waiting for Release CI..."
echo "  (this takes ~5 minutes: test + build 5 platform targets)"

# Find the release run
sleep 10
RUN_ID=$(gh run list --limit 5 --json databaseId,headBranch,event --jq '[.[] | select(.headBranch == "'"${TAG}"'")][0].databaseId')

if [[ -z "$RUN_ID" || "$RUN_ID" == "null" ]]; then
    warn "Could not find Release CI run. Check manually: gh run list"
else
    echo "  Run ID: ${RUN_ID}"
    gh run watch "${RUN_ID}" --exit-status 2>&1 | while read -r line; do
        echo "  ${line}"
    done
    ok "Release CI passed"
fi

# ── Update Homebrew formula ───────────────────────────────────────
step "Updating Homebrew formula..."

# Download tarball and compute SHA256
TARBALL_URL="https://github.com/${REPO}/archive/refs/tags/${TAG}.tar.gz"
SHA256=$(curl -sL "$TARBALL_URL" | shasum -a 256 | awk '{print $1}')
echo "  SHA256: ${SHA256}"

# Build new formula
NEW_FORMULA=$(cat <<RUBY
class AmneziaXrayAdmin < Formula
  desc "Personal CLI + Telegram bot for a double-hop Xray VPN"
  homepage "https://github.com/${REPO}"
  url "${TARBALL_URL}"
  sha256 "${SHA256}"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "amnezia-xray-admin", shell_output("#{bin}/amnezia-xray-admin --help")
  end
end
RUBY
)

# Get current file SHA and update via API
FILE_SHA=$(gh api "repos/${TAP_REPO}/contents/Formula/amnezia-xray-admin.rb" --jq '.sha')
echo "$NEW_FORMULA" | gh api "repos/${TAP_REPO}/contents/Formula/amnezia-xray-admin.rb" \
    --method PUT \
    -f message="Update amnezia-xray-admin to ${TAG}" \
    -f content="$(echo "$NEW_FORMULA" | base64)" \
    -f sha="$FILE_SHA" \
    --jq '.commit.message' >/dev/null

ok "Homebrew formula updated to ${TAG}"

# ── Done ──────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}  Released ${TAG} successfully!${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo "  GitHub:   https://github.com/${REPO}/releases/tag/${TAG}"
echo "  Homebrew: brew upgrade amnezia-xray-admin"
echo ""
