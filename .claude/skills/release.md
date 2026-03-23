# Release

Ship a new version. Runs tests, bumps version, updates CHANGELOG, tags, pushes, waits for CI, updates Homebrew.

## Usage

```
/release           # patch bump (0.1.8 → 0.1.9)
/release minor     # minor bump (0.1.8 → 0.2.0)
/release major     # major bump (0.1.8 → 1.0.0)
```

## Steps

Run `./scripts/release.sh` with the argument passed by the user (default: no argument = patch).

```bash
./scripts/release.sh $ARGUMENTS
```

If the script is not found, tell the user to run from the repo root directory.
