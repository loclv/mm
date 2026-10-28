# CHANGELOG

## 2026-10-28

### Changes

- Enhanced current repository path sanitization in copied AI Agent chat logs to automatically replace occurrences of the current path (such as `~/w/auto-commit/rust/`, raw paths, quoted paths, delimiter-bounded paths, or file URLs) with `./`, including when executing from subdirectories and across all chat providers.

## 2026-10-27

### Changes

- Auto-formatted AI Agent chat subjects according to commitlint rules (inferring missing conventional commit prefixes from repository file changes, stripping trailing full stops, and adjusting header casing and length limits).
- Added support for copying commit messages from OpenAI Codex session transcripts (`--codex` and `--codex-session`).
- Added support for copying commit messages from Claude Code session transcripts (`--claude` and `--claude-session`).
- Replaced references to the current repository path (such as `~/w/auto-commit/rust/`) with `./` in commit messages copied from AI Agent logs.
- Replaced absolute user file paths (`file:///Users/user/...`) with `~/...` when copying commit messages from AI Agent logs to prevent leaking local user names.

## 2026-10-25

### Changes

- Updated commit message generation to preserve relative file paths when creating, updating, or deleting files.
- Added automatic fallback to the deterministic, path-based auto commit message when copying logs from AI Agents fails.
- Added support for extracting commit messages from Antigravity's conversation log.
  - Added `agy` as an alias for `antigravity`.
