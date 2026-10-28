# AGENTS.md

`mm` is a single-crate Rust CLI that generates a deterministic, path-based
Conventional Commit message from Git changes and commits it. No AI or network
calls; keep it that way. Package: `mm`; binary: `mm`; library: `mm`.

## Commands

Run from the repository root (this directory):

- `./test.sh` — full gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- `cargo test --test cli <substring>` — one integration test, e.g. `cargo test --test cli commits_staged_changes`.
- `cargo test --lib <substring>` — one unit test.
- `cargo build --release` — binary at `target/release/mm`.

Unit tests live in `#[cfg(test)]` modules beside the code; end-to-end tests in
`tests/cli.rs` execute the real binary against temporary Git repositories.

## Orientation

Pipeline (path-based): `cli::run` → `Git::discover` → `Git::changes` →
`config::load` → `commitlint::load` → `Policy` → `generate_msg_with_policy` →
`Git::commit`.

Pipeline (`--chat`): `cli::run` → `Git::discover` → `Git::changes` →
`chat::load` → (format subject with `Policy` if conventional commits enabled) → `Git::commit`.
Chat mode falls back to the full path-based pipeline if copying from chat fails.

- `src/cli.rs` parses arguments by hand (no clap). A new option needs fields in
  `Options`, a `parse_args` arm, `HELP`, and tests.
- Change detection uses `git diff-index --name-status -z`; modules mirror the
  original TypeScript implementation (mapping table in README).
- User-facing CLI reference is `docs/usage.md`; keep its option table and
  behavior in sync with `HELP` in `src/cli.rs`.
- `config.rs` reads `mm.json` from the current directory upward, stopping at
  `$HOME`. `conventionalCommits` defaults to `true`; setting it to `false`
  switches generation to plain descriptions. `autoAdd` defaults to `false`;
  setting it to `true` automatically stages all changes (including untracked files)
  and commits when no changes are staged. `MessageFormat` in `generate.rs`
  implements the format switch, with `chore` as the fallback type for unknown or mixed
  paths.
- `conventional.rs` picks types from paths, checking specific categories before
  general ones. `message.rs` owns wording, `policy.rs` translates commitlint
  rules, and `commitlint.rs` handles discovery/loading.
- `chat.rs` loads chat context from OpenCode, Antigravity, Claude Code, or Codex.
  - OpenCode: shells out to `opencode session export <id>` and parses its JSON.
    `--chat` uses the session title as the subject and copies the last assistant
    message as the body, matching OpenCode's `/copy`; a completed compaction
    summary wins when it is the last message. Only `text` parts are joined with
    newlines; tool calls, reasoning and system messages are skipped.
    `OPENCODE_SESSION_ID` selects the session, `--chat-session` overrides it.
  - Antigravity: parses JSON Lines transcripts from
    `~/.gemini/antigravity-ide/brain/<id>/.system_generated/logs/transcript.jsonl`.
    `--agy` / `--antigravity` forces Antigravity mode; `--agy-session` selects a
    specific conversation ID or file path. Auto-detection prioritizes logs
    matching the repository path. The subject is the first line of the latest user
    request (from `<USER_REQUEST>`, stripped of `@` file mentions), falling back to
    the first line of the assistant message. The body is the text content from the
    last assistant message (`PLANNER_RESPONSE`).
  - Claude Code: parses JSON Lines transcripts from
    `~/.claude/projects/<encoded-project-path>/<session-id>.jsonl` (or `CLAUDE_CONFIG_DIR`).
    `--claude` forces Claude Code mode; `--claude-session` selects a specific session
    ID or file path. Auto-detection matches the repository working directory.
    The subject is the first line of the latest user prompt (ignoring tool results),
    falling back to the first line of the assistant message. The body is the text content
    from the last assistant message.
  - Codex: parses JSON Lines transcripts from
    `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl.zst` (or `.jsonl`, configured via `CODEX_HOME`).
    `--codex` forces Codex mode; `--codex-session` selects a specific session ID or file path.
    Auto-detection matches the repository working directory (via `session_meta.payload.cwd`).
    The subject uses `thread_name` from `session_index.jsonl` if available, falling back
    to the first line of the latest user prompt, falling back to the assistant message.
    The body is the text content from the last assistant message or task completion.
  - Devin / Codeium: parses ATIF JSON transcripts, JSONL logs, or exported Markdown trajectories
    from `~/.local/share/devin/cli/transcripts`, `~/.config/devin`, `~/.devin`, `~/.codeium/windsurf/cascade`,
    or configured via `DEVIN_HOME` and `DEVIN_SESSION_ID`.
    `--devin` / `--codeium` forces Devin mode; `--devin-session` / `--codeium-session` selects a
    specific session ID or file path. Auto-detection prioritizes logs matching the repository path.
    The subject is the session title or first line of the latest user prompt, falling back to the assistant message.
    The body is the text content from the last assistant message (excluding thinking blocks and tool calls).
  - `--message` overrides the copied subject. A session with no copyable text is
    an error.
  - User file URIs (`file:///Users/user/...` or `file:///home/user/...`) are
    replaced with `~/...` when copying from logs to prevent leaking local user names.
    References to the current repository directory (e.g. `~/w/auto-commit/rust/` or
    `file:///Users/user/.../rust/`) are replaced with `./`.
- `Policy` intentionally supports only rules that shape the single-line header
  (type/scope/length/full stop). In `--chat` mode, the subject header is shaped
  by Policy (inferring `docs` if all changed files are markdown, otherwise `feat`
  if the subject includes `add`/`create`/`implement` keywords, otherwise inferring
  from changes, adjusting case, stripping full stops, and enforcing length)
  when conventional commits are enabled, while the body copies the assistant
  message intact.
- Change selection is compatibility behavior: staged changes win; otherwise
  tracked worktree changes are committed with `.` as a pathspec; untracked files
  only with `--add`. `--print` must not mutate the repository (`--add` is skipped
  in print mode).

## Environment gotchas

- Integration tests need `git` on `PATH` and a writable temp directory.
- Stale build artifacts: if the repo is moved, an existing `target/` can
  retain a compiled `CARGO_BIN_EXE_mm` absolute path from the old location. The
  entire `tests/cli.rs` suite then fails with `run mm: Os { code: 2, ... }`.
  Fix with `cargo clean` (or `touch tests/cli.rs`) before rerunning.
- Node is optional. JavaScript/TypeScript commitlint configs are evaluated with
  `node`, and `extends` needs `@commitlint/load` in the target project, otherwise
  a warning is printed. Node-dependent tests silently return when Node is absent,
  so a green suite does not prove those paths ran.
- Commitlint discovery walks upward and stops at `$HOME`; JSON, YAML and
  `package.json`/`package.yaml` are parsed natively.
- `opencode session export` truncates stdout at 256 KiB when it is a pipe.
  `chat::export_session` redirects the child to a temporary file and removes it
  afterward; do not replace that with `Command::output()`.
- Chat integration tests use a fake Unix `opencode` script on `PATH`; they are
  `#[cfg(unix)]` and do not need a live OpenCode session.
- Output contract: generated message on stdout, errors as `Error: ...` on stderr,
  verbose diagnostics as `[mm] ...` on stderr. `--print` stdout is asserted
  exactly by integration tests.

## Dependencies

Only `serde_json` and `serde_norway` at runtime. Treat new dependencies as a
deliberate design decision, not a convenience.
