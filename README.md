# mm - Rust CLI

<div align="center"><svg width="88" height="88" viewBox="0 0 88 88" fill="none" xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="mm-bg" x1="0" y1="0" x2="88" y2="88" gradientUnits="userSpaceOnUse"><stop offset="0%" stop-color="#1F242C" /><stop offset="100%" stop-color="#0E1116" /></linearGradient><linearGradient id="mm-accent" x1="18" y1="24" x2="70" y2="62" gradientUnits="userSpaceOnUse"><stop offset="0%" stop-color="#F05032" /><stop offset="100%" stop-color="#FF7A59" /></linearGradient></defs><rect width="88" height="88" rx="20" fill="url(#mm-bg)" stroke="#30363D" stroke-width="1.5" /><path d="M 18 26 V 48 M 18 32 C 18 24, 29 24, 29 32 V 48 M 29 32 C 29 24, 40 24, 40 32 V 48 M 48 26 V 48 M 48 32 C 48 24, 59 24, 59 32 V 48 M 59 32 C 59 24, 70 24, 70 32 V 48" stroke="url(#mm-accent)" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round" /><path d="M 20 62 H 68" stroke="#2B313A" stroke-width="2" stroke-linecap="round" /><path d="M 29 62 H 59" stroke="url(#mm-accent)" stroke-width="2.5" stroke-linecap="round" /><circle cx="29" cy="62" r="4" fill="#15181E" stroke="url(#mm-accent)" stroke-width="2.5" /><circle cx="59" cy="62" r="4" fill="url(#mm-accent)" /></svg></div>

`mm` is a Rust rewrite of the [MichaelCurrin/auto-commit-msg](https://github.com/MichaelCurrin/auto-commit-msg) logic with:

- Copying AI Agent chat logs with only one command.
- "commitlint" rules.
- No AI, save tokens.

It generates a Conventional Commit message from Git changes and commits in one step:

```sh
mm
```

By default there is no AI or network call. The message is built from the paths
and change types that Git reports.
If the project has a commitlint configuration, its type,
scope and length rules are applied too. Inside OpenCode, Antigravity, Claude Code, Codex, Devin / Codeium, or Freebuff / Codebuff, `--chat`, `--opencode`, `--agy`, `--claude`, `--codex`, `--devin`, or `--freebuff`
can copy the session/prompt and last assistant message instead; see
[Using an OpenCode, Antigravity, Claude Code, Codex, Devin / Codeium, or Freebuff / Codebuff chat](#using-an-opencode-antigravity-claude-code-codex-devin--codeium-or-freebuff--codebuff-chat).

## Problems

- Ask AI Agents, LLMs to commit changes is waste of tokens.
- Commit messages already is logs from AI Agent.
- Simple create, update, delete a file not worth for AI Agent.

## Install

### Homebrew (macOS & Linux)

```sh
brew tap loclv/tap
brew install mm
```

### cargo-binstall (Pre-built Binaries)

Install pre-compiled binaries fast without building from source:

```sh
cargo binstall mm
```

### Pre-compiled Binaries (GitHub Releases)

Download pre-built archives for Linux (x86_64, aarch64), macOS (Intel, Apple Silicon), and Windows from [GitHub Releases](https://github.com/loclv/mm/releases).

### From Source

```sh
cargo install --path .
```

To build without installing:

```sh
cargo build --release
./target/release/mm --help
```

## Usage

```bash
mm # Generate a message and commit
mm --print # Show the message without committing
mm --add # Stage everything (including new files), then commit
mm --edit # Open the editor before committing
mm --staged # Use only staged changes
mm --chat # Copy the current OpenCode, Antigravity, Claude Code, Codex, Devin, or Freebuff chat
mm -o # Copy the current OpenCode session (same as --opencode)
mm --agy # Copy the latest Antigravity log commit message
mm --claude # Copy the latest Claude Code log commit message
mm --codex # Copy the latest Codex log commit message
mm --devin # Copy the latest Devin / Cascade log commit message
mm --freebuff # Copy the latest Freebuff / Codebuff log commit message
mm -m 'JIRA-1: fix' # Keep an existing prefix/description (or override a chat subject)
mm --completions zsh # Print shell completions (bash, zsh, or fish)
```

Run `mm --help` for the full option list. See [docs/usage.md](docs/usage.md)
for macOS/zsh installation, the complete command reference, and troubleshooting.

### Message format

Generated headers follow the
[Conventional Commits specification](https://www.conventionalcommits.org/en/v1.0.0/#specification)
by default, for example `feat: create foo.rs` or `chore: update 5 files`. When
no type can be inferred, `mm` uses `chore`.

To disable the convention, add `mm.json` in the project (the file is searched
for from the current directory upwards, stopping at your home directory):

```json
{
  "conventionalCommits": false
}
```

Messages then use plain descriptions such as `update README.md`. An explicit
`--message` prefix is still preserved, and commitlint rules still apply.

By default, `mm` automatically stages all changes (including untracked files)
when running without any staged changes. To disable this, set `"autoAdd": false` in `mm.json`:

```json
{
  "autoAdd": false
}
```

### Using an OpenCode, Antigravity, Claude Code, Codex, Devin / Codeium, or Freebuff / Codebuff chat

`mm` can extract the commit message from OpenCode, Antigravity, Claude Code, Codex, Devin / Codeium, or Freebuff / Codebuff chats:

```sh
mm --chat --print # preview the message (auto-detects OpenCode, Antigravity, Claude Code, Codex, Devin, or Freebuff)
mm --opencode --print # preview message from the current OpenCode session
mm --agy --print # preview message from the latest Antigravity log
mm --claude --print # preview message from the latest Claude Code log
mm --codex --print # preview message from the latest Codex log
mm --devin --print # preview message from the latest Devin / Cascade log
mm --freebuff --print # preview message from the latest Freebuff / Codebuff log
mm --freebuff # commit using the latest Freebuff / Codebuff log
```

- OpenCode: `OPENCODE_SESSION_ID` selects the session; `--opencode` / `-o` forces OpenCode. The subject is the session title (or latest user prompt line), and the body is the last assistant message (or compaction summary).
- Antigravity: Transcript logs from `~/.gemini/antigravity-ide/brain` (or `ANTIGRAVITY_CONVERSATION_ID`) are read. The subject is the latest user request line, and the body is the last assistant response.
- Claude Code: Transcript logs from `~/.claude/projects/` (or `CLAUDE_SESSION_ID`) are read. The subject is the latest user prompt line, and the body is the last assistant text response.
- Codex: Transcript logs from `~/.codex/sessions/` (or `CODEX_SESSION_ID`) are read. The subject is the thread title from `session_index.jsonl` (or latest user prompt line), and the body is the last assistant response.
- Devin / Codeium: Devin ATIF JSON/JSONL transcripts and Codeium Windsurf Cascade trajectory markdown files from `~/.local/share/devin/cli/transcripts`, `~/.config/devin`, `~/.devin`, `~/.codeium/windsurf/cascade` (or `DEVIN_HOME`, `DEVIN_SESSION_ID`) are read. The subject is the session title or latest user prompt line, and the body is the last assistant text response.
- Freebuff / Codebuff: Chat transcripts from `~/.config/manicode/projects/<project>/chats/<chat-id>/chat-messages.json` (or `FREEBUFF_CONFIG_DIR`, `CODEBUFF_CONFIG_DIR`, `FREEBUFF_HOME`, or `FREEBUFF_SESSION_ID`) are read. The subject is the first user prompt line (or session title from `chat-meta.json`), and the body is the last assistant text response.
- `--agy-session <ID>`, `--claude-session <ID>`, `--codex-session <ID>`, `--devin-session <ID>`, `--freebuff-session <ID>`, or `--chat-session <ID>` selects a specific session or transcript path.
- `--message` overrides the subject; the copied message remains the body.
- User file URIs like `file:///Users/user/...` are sanitized to `~/...` to prevent leaking local user names, and references to the current repository directory are sanitized to `./`.
- When Conventional Commits are enabled, commitlint rules shape the copied chat subject: inferring a missing conventional `type:` prefix from git changes, formatting casing, stripping forbidden trailing full stops, and fitting length limits. The assistant body is copied intact.
- If copying the log from the AI Agent fails, `mm` automatically falls back to generating the commit message from Git changes.

### How changes are selected

1. If anything is staged, only staged changes are used.
2. Otherwise, tracked changes in the working tree are used and the commit is run
   as `git commit -m MSG .`.
3. Untracked files are ignored unless `--add` is passed.

This matches the original `gacm` behaviour. Use `--print` when you want to see
the message without changing the repository.

### commitlint configuration

`mm` reads the project's commitlint configuration and applies the rules which
affect message generation. All formats supported by commitlint are handled:

- `package.json` or `package.yaml` with a `commitlint` field
- `.commitlintrc`, `.commitlintrc.json`, `.commitlintrc.yaml`, `.commitlintrc.yml`
- `.commitlintrc.js`, `.commitlintrc.cjs`, `.commitlintrc.mjs`
- `.commitlintrc.ts`, `.commitlintrc.cts`, `.commitlintrc.mts`
- `commitlint.config.js`, `commitlint.config.cjs`, `commitlint.config.mjs`
- `commitlint.config.ts`, `commitlint.config.cts`, `commitlint.config.mts`

The file is searched for from the current directory upwards, stopping at your
home directory, in the same order as commitlint (cosmiconfig). Use
`mm --config path/to/file` to point at a specific file.

JSON and YAML configs are parsed natively. JavaScript and TypeScript configs are
evaluated with Node, and `extends` presets are resolved with `@commitlint/load`
when it is installed. If it is not installed, the config file is imported
directly and a warning is printed because presets cannot be resolved.

Applied rules:

| Rule | Effect |
|---|---|
| `type-enum` | Restricts the generated type; falls back to an allowed type |
| `type-case` | Changes the type case, e.g. `upper-case` gives `FEAT` |
| `type-empty` | Requires a type, or removes it when it must be empty |
| `type-max-length` | Chooses a shorter type or truncates |
| `scope-enum`, `scope-empty` | Keeps or drops a scope such as `deps` |
| `scope-case` | Changes the scope case |
| `header-max-length` | Falls back to counted descriptions, then truncates |
| `subject-max-length` | Truncates the subject |
| `header-full-stop`, `subject-full-stop` | Removes a forbidden trailing full stop |

Rules about bodies, footers, commit trailers and the case of free-form subjects
are not applied to generated messages, because `mm` writes only a header. An
explicit `--message` value is treated as an override and its prefix is kept.
In chat modes (`--chat`, `--opencode`, `--agy`, `--claude`, `--codex`, `--devin`), header shaping rules
(type prefix, casing, length, and trailing full stop removal) are applied to the
subject, while copying the assistant body intact.

```sh
# .commitlintrc.yml
# rules:
# type-enum: [2, always, [feat, fix, chore]]
# header-max-length: [2, always, 72]
mm --print
```

### Message rules

- One file: `feat: create foo.rs`, `docs: update README.md`,
  `chore: delete old.txt`, `chore: rename a.rs to b.rs`
- Two to four files: names are listed grouped by action, e.g. `update src/main.rs and src/lib.rs`
  or `docs: update guide.md and create api.md`
- Five or more files: counts are used, e.g. `update 5 files`
- The type is inferred from the changed paths. When changes disagree or a type
  cannot be determined, the default is `chore`
- Prefixes come from the path: docs (`README`, `docs/`, `.rst`), tests, CI,
  dependencies (`build(deps)`), build files, and configuration (`chore`)
- A new file with no other match is a `feat`
- Renames, moves and deletions are `chore`
- `--message` keeps your existing type prefix and appends your description after
  the generated one

## Development

```sh
./test.sh
```

Or run individual commands:

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

The crate only depends on `serde_json` and `serde_norway` for reading
configuration files, and is split into modules which mirror the original
TypeScript files:

| Module | Original |
|---|---|
| `git.rs` | `git/cli.ts`, `git/parseOutput.ts` |
| `paths.rs` | `lib/paths.ts`, `lib/commonPath.ts` |
| `conventional.rs` | `generate/convCommit.ts`, `generate/convCommitConstants.ts` |
| `message.rs` | `generate/action.ts`, `generate/count.ts`, `generate/message.ts` |
| `generate.rs` | `prepareCommitMsg.ts`, `generate/parseExisting.ts` |
| `cli.rs` | `cli/diffIndexGenerate.ts`, `cli/diffIndexGenerateCommit.ts` |
| `config.rs` | New: reads `mm.json` settings |
| `chat.rs` | New: copies a title and last message from an OpenCode session |
| `commitlint.rs` | New: finds and loads the project's commitlint configuration |
| `policy.rs` | New: applies commitlint rules to a generated message |
| `completions.rs` | New: generates bash, zsh, and fish completion scripts |

## Differences from `gacm`

The selection of changes and the path-based message rules follow `gacm`. These
parts differ on purpose:

- Messages use Conventional Commits by default, so a `chore` type is added when
  no type can be inferred. Set `"conventionalCommits": false` in `mm.json` to
  restore plain descriptions.
- `--chat` can copy the title and last assistant message from an OpenCode session
  instead of deriving the message from file paths.
- The commit editor is off by default; pass `--edit` to open it. `-n` and
  `--no-edit` are accepted so existing scripts keep working.
- `--print` (`--dry-run`) prints the message and never touches the repository.
- Paths are read with `git diff-index --name-status -z`, so spaces, quotes and
  unicode in file names do not need escaping.
- A repository without commits yet is supported by diffing against the empty
  tree.

## License

MIT
