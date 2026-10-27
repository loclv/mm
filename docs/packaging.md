# Packaging & Distribution

This document details distribution channels, pre-compiled binary releases, and packaging for `mm`.

## Installation Methods

### 1. Homebrew Tap (macOS & Linux)

Install pre-compiled binaries via Homebrew:

```sh
brew tap loclv/tap
brew install mm
```

Or install directly from this repository:

```sh
brew install loclv/mm/mm
```

To upgrade:

```sh
brew upgrade mm
```

### 2. cargo-binstall (Cross-platform)

[`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall) fetches pre-compiled binaries directly from GitHub Releases without requiring a local Rust compilation toolchain:

```sh
cargo binstall mm
```

### 3. Pre-compiled Binaries (GitHub Releases)

Download pre-built archives from [GitHub Releases](https://github.com/loclv/mm/releases):

| Target Triple | OS / Architecture | Archive Format |
|---|---|---|
| `x86_64-unknown-linux-gnu` | Linux 64-bit (x86_64) | `.tar.gz` |
| `aarch64-unknown-linux-gnu` | Linux 64-bit ARM (aarch64) | `.tar.gz` |
| `x86_64-apple-darwin` | macOS Intel (x86_64) | `.tar.gz` |
| `aarch64-apple-darwin` | macOS Apple Silicon (arm64) | `.tar.gz` |
| `x86_64-pc-windows-msvc` | Windows 64-bit (x86_64) | `.zip` |

Each release also publishes `SHA256SUMS.txt` for integrity verification.

Example manual install on macOS / Linux:

```sh
curl -L -O https://github.com/loclv/mm/releases/latest/download/mm-aarch64-apple-darwin.tar.gz
tar -xzf mm-aarch64-apple-darwin.tar.gz
chmod +x mm
sudo mv mm /usr/local/bin/
```

### 4. Build from Source

```sh
cargo install --path .
# or from crates.io
cargo install mm
```

---

## Release Pipeline (GitHub Actions)

The release workflow is located at [`.github/workflows/release.yml`](file:///Users/a0/w/auto-commit/rust/.github/workflows/release.yml).

### Trigger

The pipeline triggers automatically whenever a tag matching `release-*` is pushed:

```sh
git tag -a release-v0.2.3 -m "Release v0.2.3"
git push origin release-v0.2.3
```

*(Note: Tags named `release-0.2.3` are automatically aliased to `release-v0.2.3` in CI to maintain compatibility with `cargo-binstall`'s SemVer prefix).*

### Pipeline Steps

1. **Create GitHub Release**: Generates a GitHub Release corresponding to the git tag.
2. **Cross-Compilation Matrix**:
   - `x86_64-unknown-linux-gnu` on `ubuntu-latest`
   - `aarch64-unknown-linux-gnu` on `ubuntu-latest` via `cross`
   - `x86_64-apple-darwin` on `macos-13` (Intel runner)
   - `aarch64-apple-darwin` on `macos-latest` (Apple Silicon runner)
   - `x86_64-pc-windows-msvc` on `windows-latest`
3. **Archive Generation**:
   - Compiles release binaries with LTO, codegen-units=1, and stripped symbols.
   - Archives into `mm-<target>.tar.gz` (Unix) and `mm-<target>.zip` (Windows) including `README.md` and `LICENSE`.
   - Uploads binary assets to the release.
4. **Homebrew & Checksums Automation**:
   - Downloads all release assets and calculates SHA256 checksums into `SHA256SUMS.txt`.
   - Generates multi-architecture `Formula/mm.rb` supporting both Apple Silicon / Intel macOS and ARM64 / x86_64 Linux.
   - Uploads `SHA256SUMS.txt` and `Formula/mm.rb` to the release.
   - If `HOMEBREW_TAP_TOKEN` secret is configured in the repository, automatically commits and pushes the updated formula to the Homebrew tap repository (default `loclv/homebrew-tap`).

### Homebrew Tap Setup

To enable automated formula updates when a new release is cut:
1. Create a GitHub Personal Access Token (PAT) with `repo` or `Contents: write` permissions for your tap repository.
2. In the `loclv/mm` GitHub repository settings, go to **Settings > Secrets and variables > Actions**.
3. Add secret `HOMEBREW_TAP_TOKEN`.
4. (Optional) Set repository variable `HOMEBREW_TAP_REPO` if your tap repository name differs from `loclv/homebrew-tap`.
