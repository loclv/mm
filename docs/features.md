# Features

`mm` is a single-crate, high-performance Rust CLI that generates deterministic, path-based Conventional Commit messages from Git changes or extracts rich commit context directly from local AI agent transcripts. It requires **zero AI queries, zero network calls, and zero external tokens**.

---

## 1. Deterministic Path-Based Commit Generation

When run in standard mode (`mm`), the CLI inspects local repository changes via Git (`git diff-index --name-status -z`) and determines the commit message based on modified, created, or deleted file paths.

- **Heuristic Type Mapping**:
  - Automatically identifies Conventional Commit types based on path conventions:
    - Code & features (`src/`, `lib/`, `app/`): `feat`
    - Tests (`tests/`, `test/`, `*_test.rs`, `*.spec.ts`): `test`
    - Documentation (`docs/`, `*.md`, `LICENSE`): `docs`
    - Build systems & dependencies (`Cargo.toml`, `package.json`, `Makefile`, `Dockerfile`): `build`
    - CI/CD workflows (`.github/workflows/`, `.gitlab-ci.yml`): `ci`
    - Styles & formatters (`.editorconfig`, `rustfmt.toml`, `.prettierrc`): `style`
    - General maintenance & multi-directory updates: `chore`
- **Smart Counting & Single-File Naming**:
  - Concise single-file subjects: `feat: create src/cli.rs` or `docs: update README.md`.
  - Counted multi-file summaries: `chore: update 5 files`.
- **Change Selection & Automatic Staging**:
  - Prioritizes staged changes when present.
  - Automatically stages all changes (including untracked files) by default (`autoAdd: true`).
  - Restrict to staged-only changes using `-s` / `--staged` or `"autoAdd": false`.
- **Format Toggle**:
  - Conventional Commits enabled by default (`feat: ...`).
  - Can be switched to plain human-readable messages via `"conventionalCommits": false` in `mm.json`.

---

## 2. Multi-Agent Chat Transcript Integration (`--chat`)

For AI-assisted workflows, `mm --chat` copies session titles, user prompts, and assistant summaries directly into Git commit messages:

- **Subject**: Uses the chat title or the first line of the latest user prompt, shaped by commitlint policy into a Conventional Commit header.
- **Body**: Copies the assistant's final response or task completion summary.
- **Overriding**: Pass `-m "custom message"` to override the copied subject while preserving the transcript body.

### Supported AI Agent Providers

| Provider | Flags | Transcript Discovery & Matching |
|---|---|---|
| **Antigravity** | `--agy`, `--antigravity`, `--agy-session <ID>` | Discovers transcripts across `~/.gemini/antigravity/brain`, `antigravity-ide`, `antigravity-cli`, or `ANTIGRAVITY_BRAIN_DIR`. Queries `conversation_summaries.db` workspace URIs and parses `transcript.jsonl`. |
| **Claude Code** | `--claude`, `--claude-session <ID>` | Resolves project folders via encoded paths and `history.jsonl` under `~/.claude/projects/` or `CLAUDE_CONFIG_DIR`. |
| **Codex** | `--codex`, `--codex-session <ID>` | Discovers session rollouts (`.jsonl`, `.jsonl.zst`) in `~/.codex/sessions/` or `CODEX_HOME`, matching `cwd` and `session_index.jsonl` thread names. |
| **Devin / Cascade** | `--devin`, `--codeium`, `--devin-session <ID>` | Parses ATIF JSON, JSONL logs, or Markdown trajectories in `~/.local/share/devin/cli/transcripts`, `~/.config/devin`, or `~/.codeium/windsurf/cascade`. |
| **Freebuff / Codebuff** | `--freebuff`, `--codebuff`, `--freebuff-session <ID>` | Reads `chat-messages.json` and `chat-meta.json` from `~/.config/manicode/projects/` or repository-local `.freebuff/chats/`. |
| **OpenCode** | `-o`, `--opencode`, `--chat-session <ID>` | Shells out to `opencode session export <id>`, prioritizing completed compaction summaries and text blocks. |

---

## 3. Real-Time Multi-Project Path Auto-Detection

When working across multiple repositories simultaneously in real time, `mm` accurately pairs each repository with its corresponding AI session across all supported agent harnesses:

- **Antigravity**: Resolves sessions via `conversation_summaries.db` workspace URIs and streams `transcript.jsonl` matching the repository path.
- **OpenCode**: Auto-detects matching sessions by checking OpenCode storage project configurations (`storage/project/*.json` worktrees) and CLI session lists against the repository root.
- **Claude Code**: Matches encoded project directories (`~/.claude/projects/<encoded-path>`), `history.jsonl` project records, and recursive session transcript contents.
- **Codex**: Matches the repository working directory in `session_meta.payload.cwd`, `session_index.jsonl` directory mappings, and deep streaming scans of `.jsonl`/`.jsonl.zst` rollouts.
- **Devin / Cascade**: Inspects session metadata worktrees and scans recent transcripts for the repository root across ATIF JSON, JSONL logs, and Markdown trajectories.
- **Freebuff / Codebuff**: Resolves repository IDs via `.freebuff/project-id` / `.codebuff/project-id`, project folder names under `~/.config/manicode/projects/`, `chat-meta.json`, and deep chat transcript scans.

### Robust Path Scoping & Matching

- **Repository Root Scoping**: Evaluates the Git top-level root (`git rev-parse --show-toplevel`) against conversation workspace metadata.
- **Deep Path Inspection**: Checks project paths against:
  - Canonical and symlinked paths (such as macOS `/var` vs `/private/var`).
  - Home-relative tilde paths (`~/...`).
  - Windows forward-slash and backslash variants.
  - JSON-escaped paths and `file:///` URIs.
- **Cross-Agent Priority**: Prioritizes sessions matching the current repository's working directory before falling back to cross-project timestamps.
- **Full Transcript Scanning**: Streams transcripts line-by-line (up to 2MB) to identify repository mentions even in lengthy conversations with large initial contexts.

---

## 4. Smart Body Sanitization & Privacy Protection

`mm` cleans and sanitizes chat transcripts before generating commit bodies:

- **Conversational Filler Removal**:
  - Strips leading conversational prefixes (e.g. *"Sure! I will help you with..."*, *"Here is a summary of changes:"*).
  - Trims polite sign-offs and conversation closings.
- **Tool Residue & Markup Stripping**:
  - Removes XML tool call fences (e.g. `<tool_call>`, `<edit_file>`, `<read_file>`).
  - Strips markdown code blocks containing CLI and runner logs.
  - Cleans ANSI escape sequences.
- **Privacy & User Path Redaction**:
  - Replaces user home directories (`file:///Users/username/...` or `file:///home/username/...`) with `~/...` to prevent username leakage.
  - Rewrites absolute paths to the current repository into clean relative paths (`./...`).

---

## 5. Commitlint Rules & Policy Enforcement

`mm` discovers and respects existing project `commitlint` configurations:

- **Zero-Dependency Native Parsing**: Directly parses JSON, YAML, and `package.json` commitlint configurations without requiring Node.js.
- **Node.js Fallback**: Seamlessly evaluates JavaScript/TypeScript configurations (`commitlint.config.js`, `commitlint.config.mjs`, `commitlint.config.ts`) and `@commitlint/load` extensions when Node is available.
- **Enforced Rules**:
  - `type-enum`: Restricts header types to allowed categories.
  - `type-case`, `scope-case`, `subject-case`: Applies lowercase, uppercase, or sentence-case rules.
  - `header-max-length` & `subject-max-length`: Truncates subjects safely at word boundaries.
  - `header-full-stop`: Ensures headers omit or include trailing periods per configuration.

---

## 6. Flexible Configuration (`mm.json`)

`mm` searches upward from the current directory to `$HOME` for an optional `mm.json` config:

```json
{
  "conventionalCommits": true,
  "autoAdd": true,
  "config": "./custom-commitlint.json"
}
```

- `conventionalCommits` (boolean, default: `true`): Toggle Conventional Commit prefixes.
- `autoAdd` (boolean, default: `true`): Automatically stage untracked files when no changes are currently staged.
- `config` (string, optional): Point directly to a custom commitlint configuration file.

---

## 7. CLI Ergonomics & Developer Workflow

- **Preview Mode (`-p`, `--print`, `-d`, `--dry-run`)**: Output generated messages to stdout without making any Git commit or repository changes.
- **Interactive Editing (`-e`, `--edit`)**: Open the generated message in `$GIT_EDITOR` before committing.
- **Bypass Git Hooks (`--no-verify`)**: Pass `--no-verify` to `git commit` to bypass pre-commit or commit-msg hooks when needed.
- **Shell Auto-Completions (`--completions <SHELL>`)**: Generate native completion scripts for `bash`, `zsh`, or `fish`.
- **Diagnostic Logging (`--verbose`)**: Print detailed change detection, session resolution, and commitlint matching steps to stderr.
