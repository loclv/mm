# AGENTS.md

`ac` is a single-crate Rust CLI that generates a deterministic, path-based
Conventional Commit message from Git changes and commits it. No AI or network
calls; keep it that way. Package: `auto-commit-msg`; binary: `ac`; library:
`auto_commit_msg`.

## Commands

Run from the repository root (this directory):

- `./test.sh` — full gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- `cargo test --test cli <substring>` — one integration test, e.g. `cargo test --test cli commits_staged_changes`.
- `cargo test --lib <substring>` — one unit test.
- `cargo build --release` — binary at `target/release/ac`.

Unit tests live in `#[cfg(test)]` modules beside the code; end-to-end tests in
`tests/cli.rs` execute the real binary against temporary Git repositories.

## Orientation

Pipeline: `cli::run` → `Git::discover` → `Git::changes` → `commitlint::load` →
`Policy` → `generate_msg_with_policy` → `Git::commit`.

- `src/cli.rs` parses arguments by hand (no clap). A new option needs fields in
  `Options`, a `parse_args` arm, `HELP`, and tests.
- Change detection uses `git diff-index --name-status -z`; modules mirror the
  original TypeScript implementation (mapping table in README).
- `conventional.rs` picks types from paths, checking specific categories before
  general ones. `message.rs` owns wording, `policy.rs` translates commitlint
  rules, and `commitlint.rs` handles discovery/loading.
- `Policy` intentionally supports only rules that shape the single-line header
  (type/scope/length/full stop). Bodies, footers and trailers are never
  generated.
- Change selection is compatibility behavior: staged changes win; otherwise
  tracked worktree changes are committed with `.` as a pathspec; untracked files
  only with `--add`. `--print` must not mutate the repository (`--add` is skipped
  in print mode).

## Environment gotchas

- This crate is the repository root; run Cargo here, not from a parent `rust/`
  subdirectory.
- Integration tests need `git` on `PATH` and a writable temp directory.
- Stale build artifacts: if the repo is moved, an existing `target/` can
  retain a compiled `CARGO_BIN_EXE_ac` absolute path from the old location. The
  entire `tests/cli.rs` suite then fails with `run ac: Os { code: 2, ... }`.
  Fix with `cargo clean` (or `touch tests/cli.rs`) before rerunning.
- Node is optional. JavaScript/TypeScript commitlint configs are evaluated with
  `node`, and `extends` needs `@commitlint/load` in the target project, otherwise
  a warning is printed. Node-dependent tests silently return when Node is absent,
  so a green suite does not prove those paths ran.
- Commitlint discovery walks upward and stops at `$HOME`; JSON, YAML and
  `package.json`/`package.yaml` are parsed natively.
- Output contract: generated message on stdout, errors as `Error: ...` on stderr,
  verbose diagnostics as `[ac] ...` on stderr. `--print` stdout is asserted
  exactly by integration tests.

## Dependencies

Only `serde_json` and `serde_norway` at runtime. Treat new dependencies as a
deliberate design decision, not a convenience.
