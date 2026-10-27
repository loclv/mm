# Release Process

This document details the checklist and steps for releasing a new version of `mm`.

## 1. Quality Gate & Pre-checks

Always ensure the entire test suite, code formatting, and clippy pass cleanly before preparing a release:

```sh
./test.sh
```

`./test.sh` runs:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test` (unit tests and integration tests in `tests/cli.rs`)

Do not proceed if any check fails.

## 2. Version Bump

Update the version number in [Cargo.toml](file:///Users/a0/w/auto-commit/rust/Cargo.toml):

```toml
[package]
name = "mm"
version = "x.y.z" # bump accordingly (SemVer)
```

Then check that Cargo recognizes the updated dependencies and lockfile:

```sh
cargo check
```

## 3. Update Changelog

Document new features, fixes, and behavioral changes in [CHANGELOG.md](file:///Users/a0/w/auto-commit/rust/CHANGELOG.md):

```markdown
## x.y.z - YYYY-MM-DD

### Features

- Added `--agy` / `--antigravity` to extract commit messages from Antigravity conversation logs.
- Added auto-detection for Antigravity conversation transcripts under `--chat`.
- Added automatic fallback to path-based commit messages when AI agent chat extraction fails.
```

## 4. Build & Verify Release Binary

Compile an optimized release binary:

```sh
cargo build --release
```

Verify that the compiled binary reports the new version and includes the expected flags:

```sh
./target/release/mm -v
./target/release/mm --help
```

## 5. Update Local Installation

>[!IMPORTANT]
>Running `cargo build --release` only creates `./target/release/mm`.
>If `which mm` points to `~/.cargo/bin/mm`, running `mm` in your terminal will continue executing the old version until you install the updated binary.

To update your global CLI executable:

```sh
cargo install --path . --force
```

If you are using zsh, refresh the shell's command path hash table:

```sh
rehash
```

Confirm that the global command is up to date:

```sh
which mm # should show ~/.cargo/bin/mm
mm -v # should show the newly bumped version
```

## 6. Commit & Tag

Commit the release changes:

```sh
git add Cargo.toml Cargo.lock CHANGELOG.md docs/ Formula/
git commit -m "chore: release v0.2.3"
```

Create an annotated Git tag matching `release-*` (for example `release-v0.2.3`):

```sh
git tag -a release-v0.2.3 -m "Release v0.2.3"
```

Push commits and tags to remote:

```sh
git push origin main
git push origin release-v0.2.3
# or push both at once:
git push origin main --tags
```

## 7. Automated Cross-Compilation & Release Pipeline

Pushing a `release-*` tag triggers [`.github/workflows/release.yml`](file:///Users/a0/w/auto-commit/rust/.github/workflows/release.yml):

- Cross-compilation: Compiles binaries for:
  - Linux `x86_64` (`x86_64-unknown-linux-gnu`)
  - Linux `aarch64` (`aarch64-unknown-linux-gnu`)
  - macOS Intel (`x86_64-apple-darwin`)
  - macOS Apple Silicon (`aarch64-apple-darwin`)
  - Windows `x86_64` (`x86_64-pc-windows-msvc`)
- Assets uploaded: Release tarballs (`.tar.gz`), zip (`.zip`), and `SHA256SUMS.txt`.
- `cargo-binstall`: Supported out-of-the-box via `[package.metadata.binstall]` in `Cargo.toml`.
- Homebrew tap: Automatically generates `Formula/mm.rb` with current checksums and pushes to `loclv/homebrew-tap` if `HOMEBREW_TAP_TOKEN` secret is configured.

See [docs/packaging.md](file:///Users/a0/w/auto-commit/rust/docs/packaging.md) for full distribution and packaging details.

## 8. Publishing (crates.io)

If publishing to [crates.io](https://crates.io):

1. Perform a dry-run publish first:
   ```sh
   cargo publish --dry-run
   ```
2. Publish:
   ```sh
   cargo publish
   ```
