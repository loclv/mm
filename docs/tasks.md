# Tasks & Roadmap Ideas

This document collects planned tasks, feature ideas, and architectural enhancements for `mm`.

`mm` is a fast, deterministic, path-based Conventional Commit CLI built in Rust. It operates with zero AI and zero network calls, generating structured commits from file changes and extracting commit messages from AI agent transcripts (OpenCode, Antigravity, Claude Code, Codex).

## 1. AI Agent Transcripts & Providers

Expand and refine transcript parsing for developer AI environments.

- [ ] Cursor IDE Support `[Priority: High]`
  - Parse chat transcripts from Cursor workspace storage / state databases.
  - Auto-detect when working inside Cursor sessions.
  - CLI flags: `--cursor`, `--cursor-session <ID>`.
- [ ] Devin / Codeium Support `[Priority: Medium]`
  - Support reading Cascade / Devin transcript logs.
  - CLI flags: `--devin`, `--devin-session <ID>`.
- [ ] Aider Chat History Support `[Priority: Medium]`
  - Extract the latest user prompt and assistant explanation from `.aider.chat.history.md`.
  - CLI flags: `--aider`.
- [ ] Roo Code / Cline Transcript Support `[Priority: Medium]`
  - Parse tasks from VS Code global storage for Roo Code / Cline tasks.
  - CLI flags: `--cline`, `--roo`.
- [ ] Smart Body Sanitization & Noise Reduction `[Priority: High]`
  - Strip conversational filler from assistant responses (e.g. "Sure! I will help you with...", "Here is a summary:").
  - Strip tool call residue and ANSI escape sequences if present.
  - Support `--max-body-lines <N>` to prevent bloated commit logs on massive multi-file refactor outputs.
- [ ] Co-Author & Agent Git Trailers `[Priority: Medium]`
  - Automatically append configurable trailer in chat mode (e.g. `Co-authored-by: Antigravity <agent@antigravity.google>`).
  - Flag `--co-author` or `"trailers": [...]` setting in `mm.json`.

## 2. Conventional Commit Heuristics & Scopes

Enhance path-based commit message generation for modern project layouts.

- [ ] Monorepo Package & Crate Scope Detection `[Priority: High]`
  - Detect subprojects in Cargo workspaces (`crates/<name>/...`), npm/pnpm/yarn workspaces (`packages/<name>/...`), and Go modules.
  - Automatically infer scope: `feat(cli): ...`, `fix(web): ...`.
  - Configure via `"inferScope": "workspace" | "directory" | "none"` in `mm.json`.
- [ ] Fine-Grained Category Inferences `[Priority: Medium]`
  - Benchmarks: `benches/`, `benchmark/` -> `perf` or `test`.
  - Styles & Linters: `.prettierrc`, `rustfmt.toml`, `biome.json`, `.editorconfig` -> `style`.
  - Workflows & CI: `.github/workflows/`, `.gitlab-ci.yml`, `Jenkinsfile` -> `ci`.
  - Internationalization: `locales/`, `i18n/`, `*.po` -> `i18n` or `docs`.
  - Database Migrations: `migrations/`, `schema.prisma`, `alembic/` -> `feat` or `db`.
- [ ] Breaking Changes (`!`) Detection `[Priority: Low]`
  - Detect `BREAKING CHANGE:` in user prompt or commit description.
  - Generate `feat!: ...` or `refactor!: ...` header when breaking changes are indicated.

## 3. Configuration & Custom Rules (`mm.json`)

Enable flexible customization while keeping default zero-config behavior.

- [ ] Custom Path Rule Mappings `[Priority: High]`
  - Allow defining custom pattern-to-type rules in `mm.json`:
    ```json
    {
      "rules": [
        { "glob": "docs/**/*.md", "type": "docs" },
        { "glob": "proto/**/*.proto", "type": "api", "scope": "proto" }
      ]
    }
    ```
- [ ] Issue & Branch Ticket Integration `[Priority: Medium]`
  - Auto-extract issue/ticket numbers from branch names (e.g., `feat/ABC-123-auth` -> `feat(ABC-123): ...` or appending `Refs: #123`).
  - Configurable in `mm.json` via regex pattern (e.g. `"ticketPattern": "[A-Z]+-[0-9]+"`).
- [ ] Default Chat Provider Preference `[Priority: Low]`
  - Configure `"defaultChatProvider": "antigravity"` in `mm.json` so `mm --chat` explicitly prefers a specific agent without needing `--agy`.

## 4. CLI Usability & Git Workflow

Streamline command-line ergonomics and developer workflows.

- [ ] Machine-Readable Output (`--json`) `[Priority: High]`
  - Add `--json` flag to output structured commit metadata:
    ```json
    {
      "type": "docs",
      "scope": null,
      "subject": "update tasks and roadmap",
      "body": "...",
      "files": ["docs/tasks.md"]
    }
    ```
  - Useful for agent tool calling, shell scripts, and CI checks.
- [ ] Interactive Preview Mode (`-i` / `--interactive`) `[Priority: Medium]`
  - Provide an interactive prompt showing the proposed subject, body, and changed file list with `[Y/n/e]` (Commit / Cancel / Edit).
- [ ] Quick Amend (`--amend`) `[Priority: Medium]`
  - Allow amending the previous commit with newly staged/unstaged changes and an updated message without manual `git commit --amend`.
- [ ] Git Hook Generation (`mm hook install`) `[Priority: Low]`
  - Helper command to install `prepare-commit-msg` or `commit-msg` hooks for projects using git hooks.
- [ ] Shell Auto-Completions `[Priority: Low]`
  - Generate completion scripts for `bash`, `zsh`, and `fish` via `mm --completions <shell>`.

## 5. Performance, Infrastructure & Distribution

- [x] Zero-Copy Git Status Parsing `[Priority: Low]`
  - Zero-copy NUL-separated change list parsing with `Cow<'a, str>`, `unsafe` UTF-8 conversion fast paths, and `Arc<[u8]>` pinned backing buffers for massive commits (1,000+ files).
- [x] Pre-built Binaries & Packaging `[Priority: Medium]`
  - GitHub Actions release pipeline for cross-compiling Linux (x86_64, aarch64), macOS (Intel, Apple Silicon), and Windows.
  - Publish to `cargo-binstall` and Homebrew tap.
