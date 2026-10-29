# mm CLI usage

`mm` generates a Conventional Commit message from the changes Git reports, then
commits them. By default it is deterministic and path-based: no AI, no network
calls. `--chat` instead copies the title and last assistant message from a local
OpenCode session.

## Requirements

- macOS with zsh (the install steps below use Cargo)
- Git
- Rust 1.85+ (the crate uses edition 2024) when installing from source
- Optional: Node.js for JavaScript/TypeScript commitlint configs

## Install for macOS / zsh

### Option A: Homebrew (Recommended)

```sh
brew tap loclv/tap
brew install mm
```

### Option B: cargo-binstall (Pre-built Binaries)

```sh
cargo binstall mm
```

### Option C: From Source via Cargo

From the repository root:

```sh
cargo install --path . --force
```

Cargo installs the binary to `~/.cargo/bin/mm`. Confirm that Cargo's bin
directory is on your zsh `PATH` and that the command resolves:

```sh
echo "$PATH" | tr ':' '\n' | grep -F "$HOME/.cargo/bin"
command -v mm
mm --version
```

If `mm` is not found, add this to `~/.zshrc` and start a new shell:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

After reinstalling or replacing the binary, run `rehash` so zsh refreshes its
command cache.

To update after changing the source, reinstall:

```sh
cargo install --path . --force
```

To uninstall:

```sh
cargo uninstall mm
```

## Quick start

`mm` commits immediately, so preview the message first when you are unsure:

```sh
mm -p # generate and print the message only
mm # generate and commit
```

## Options

| Option | Meaning |
|---|---|
| `--completions <SHELL>` | Print a completion script for `bash`, `zsh`, or `fish`, then exit |
| `-a`, `--add` | Stage all changes, including untracked files, then commit |
| `-e`, `--edit` | Open the generated message in Git's editor before committing |
| `-p`, `--print` | Print the message and exit; never commit |
| `-d`, `--dry-run` | Alias for `--print` |
| `-m`, `--message <MSG>` | Merge the generated description with an existing message, or override the chat subject |
| `--chat` | Copy the chat title and last message (OpenCode, Antigravity, or Claude Code) |
| `--chat-session <ID>` | Use a specific session ID or transcript path; implies `--chat` |
| `--agy`, `--antigravity` | Copy the Antigravity last log commit message; implies `--chat` |
| `--agy-session <ID>` | Use a specific Antigravity conversation or transcript; implies `--agy` |
| `--claude` | Copy the Claude Code last log commit message; implies `--chat` |
| `--claude-session <ID>` | Use a specific Claude Code session or transcript; implies `--claude` |
| `--codex` | Copy the Codex last log commit message; implies `--chat` |
| `--codex-session <ID>` | Use a specific Codex session or transcript; implies `--codex` |
| `--devin`, `--codeium` | Copy the Devin / Codeium Cascade last log commit message; implies `--chat` |
| `--devin-session <ID>` | Use a specific Devin or Cascade session or transcript; implies `--devin` |
| `-c`, `--config <FILE>` | Use a specific commitlint config instead of searching |
| `-s`, `--staged` | Only use staged changes; never fall back to unstaged changes |
| `--no-verify` | Bypass Git commit hooks |
| `--verbose` | Print diagnostics to stderr |
| `-n`, `--no-edit` | Compatibility flag; editing is already off by default |
| `-h`, `--help` | Show help and exit |
| `-v`, `-V`, `--version` | Show the version and exit |

## Shell completions

`mm --completions <shell>` prints a completion script to stdout and exits. It
works outside a Git repository and does not change anything on disk, so you can
redirect the output wherever your shell loads completions from:

```sh
# bash (macOS Homebrew or Linux)
mm --completions bash > /etc/bash_completion.d/mm
mm --completions bash > ~/.local/share/bash-completion/completions/mm

# zsh
mkdir -p ~/.zfunc
mm --completions zsh > ~/.zfunc/_mm
# then add `fpath+=~/.zfunc` before `compinit` in ~/.zshrc

# fish
mm --completions fish > ~/.config/fish/completions/mm.fish
```

For zsh with Oh My Zsh, the script also works as a plugin: save it as
`$ZSH_CUSTOM/plugins/mm/mm.plugin.zsh` and add `mm` to `plugins`.

An unsupported shell name is an error:

```sh
mm --completions nushell
# Error: unsupported shell `nushell` (expected bash, zsh, fish)
```

## How changes are selected

1. If anything is staged, only staged changes are used.
2. If nothing is staged, all changes (including untracked files) are
   automatically staged and committed (default `autoAdd: true`).
3. If `autoAdd` is disabled in `mm.json` (`"autoAdd": false`), tracked working-tree
   changes are committed with `git commit -m MSG .`, and untracked files are
   ignored unless `--add` is passed.

`--staged` disables the fallback in step 2, so an empty index is an error.

`--print` never changes the repository. `--add` only stages for a real commit, so
`mm --print --add` still excludes untracked files from the preview; if only
untracked files exist, it reports that it cannot find changes.

## Generated messages

Generated headers follow the
[Conventional Commits specification](https://www.conventionalcommits.org/en/v1.0.0/#specification)
by default:

- One file: `feat: create foo.rs`, `docs: update README.md`,
  `chore: delete old.txt`, or `chore: rename a.rs to b.rs`.
- Two to four files: names are listed grouped by action, for example
  `chore: update src/main.rs and src/lib.rs` or `docs: update guide.md and create api.md`.
- Five or more files: counts are used, for example `chore: update 5 files`.
- Types come from the path: docs, tests, CI, dependencies (`build(deps)`), build
  files, and configuration (`chore`).
- A new file with no other match is a `feat`.
- Renames, moves, and deletions are `chore`.
- When no type can be inferred, or the changed paths disagree, the type is
  `chore`.

An existing type prefix supplied with `--message` wins over the generated type;
an existing description is appended after the generated description.

### Configuration with `mm.json`

Add `mm.json` to the project (or a parent directory up to `$HOME`) to configure project options:

- `conventionalCommits` (boolean, default: `true`): set to `false` (or alias `"conventional"`) to generate plain descriptions without a Conventional Commit type prefix:

```json
{
  "conventionalCommits": false
}
```

- `autoAdd` (boolean, default: `true`): by default, `mm` automatically stages and commits all changes (including untracked files) when running without any staged changes. If files are already staged, `mm` commits only the staged changes and leaves unstaged/untracked files untouched. To disable this and ignore untracked files when nothing is staged, set `"autoAdd": false` (or aliases `"add"`, `"autoCommit"`, `"autoStage"`):

```json
{
  "autoAdd": false
}
```

`mm` then generates plain descriptions such as `update README.md` when Conventional Commits are disabled; it does not
add a `chore` fallback or any `type:` prefix. An explicit `--message` prefix is still preserved, and
commitlint rules still apply.

`mm.json` is searched upward from the current directory, stopping at your home
directory, the same way commitlint configuration is found. `--config` is
unrelated: it overrides the commitlint configuration file.

## Using an OpenCode, Antigravity, Claude Code, Codex, or Devin / Codeium chat

`mm` can extract the commit message directly from local assistant chats:

- OpenCode: OpenCode exports sessions using its `opencode` CLI. When `mm` runs inside an OpenCode session, `OPENCODE_SESSION_ID` identifies the current chat.
- Antigravity: Antigravity saves conversation transcripts in `~/.gemini/antigravity-ide/brain/<ID>/.system_generated/logs/transcript.jsonl`. When running inside Antigravity (or when `ANTIGRAVITY_CONVERSATION_ID` is set), or via `--agy`, `mm` reads the conversation log.
- Claude Code: Claude Code saves session transcripts in `~/.claude/projects/<encoded-project-path>/<session-id>.jsonl` (or under `CLAUDE_CONFIG_DIR`). When running inside Claude Code (or when `CLAUDE_SESSION_ID` is set), or via `--claude`, `mm` reads the session log.
- Codex: OpenAI Codex saves session transcripts in `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl.zst` (or `.jsonl`, under `CODEX_HOME`). When running inside Codex (or when `CODEX_SESSION_ID` is set), or via `--codex`, `mm` reads the session log.
- Devin / Codeium: Devin ATIF JSON/JSONL transcripts and Codeium Windsurf Cascade trajectory markdown files from `~/.local/share/devin/cli/transcripts`, `~/.config/devin`, `~/.devin`, `~/.codeium/windsurf/cascade` (or under `DEVIN_HOME`, `DEVIN_SESSION_ID`). When running inside Devin or Windsurf Cascade, or via `--devin` / `--codeium`, `mm` reads the transcript.

```sh
mm --chat --print # preview copied message (auto-detects OpenCode, Antigravity, Claude Code, Codex, or Devin)
mm --agy --print # preview message from the latest Antigravity log
mm --claude --print # preview message from the latest Claude Code log
mm --codex --print # preview message from the latest Codex log
mm --devin --print # preview message from the latest Devin / Cascade log
mm --devin # commit using the latest Devin / Cascade log
```

The message is built like this:

- Subject:
  - For OpenCode, the session title is used (or the first line of the latest user prompt, truncated to 72 characters, if untitled).
  - For Antigravity, the first line of the latest user request (extracted from `<USER_REQUEST>` and stripped of `@` file references) is used, truncated to 72 characters. If absent, the first line of the assistant message is used.
  - For Claude Code, the first line of the latest user prompt is used, truncated to 72 characters (tool execution results are skipped). If absent, the first line of the assistant message is used.
  - For Codex, the thread title from `session_index.jsonl` is used if available; otherwise the first line of the latest user prompt is used, truncated to 72 characters. If absent, the first line of the assistant message is used.
  - For Devin / Codeium, the session title is used if available; otherwise the first line of the latest user prompt is used, truncated to 72 characters. If absent, the first line of the assistant message is used.
- Body: The last assistant response (`PLANNER_RESPONSE`, Claude Code text message, Codex assistant message/task completion, Devin assistant message, or OpenCode assistant text message). Tool calls, reasoning/thinking, and metadata are excluded.
- `--agy` or `--antigravity` forces copying from Antigravity logs.
- `--claude` forces copying from Claude Code logs.
- `--codex` forces copying from Codex logs.
- `--devin` or `--codeium` forces copying from Devin / Codeium logs.
- `--agy-session <ID>`, `--claude-session <ID>`, `--codex-session <ID>`, `--devin-session <ID>`, or `--chat-session <ID>` selects a specific session or transcript path and implies `--chat`.
- `--message` overrides the subject; the copied assistant message remains the body.
- Privacy and path sanitization: Absolute user file links (such as `file:///Users/user/...` or `file:///home/user/...`) copied from chat logs are automatically replaced with `~/...` to prevent leaking local user names in commit messages. Paths referring to the current repository directory (such as `~/w/auto-commit/rust/` or `file:///.../rust/`) are replaced with `./`.
- Commitlint prefix auto-formatting: When Conventional Commits are enabled (the default), `mm` formats the chat subject according to commitlint rules (e.g. `@commitlint/config-conventional`). If the copied subject lacks a conventional `type:` prefix, `mm` infers one (using `docs` when all changed files are markdown files, otherwise `feat` if the text includes keywords like `add`, `create`, or `implement`, otherwise from git changes), formats the subject casing, strips trailing full stops (such as `...` or `.`), and fits the header within length limits. The assistant response body is preserved.

`--chat` / `--agy` / `--claude` / `--codex` / `--devin` uses the chat subject (shaped by commitlint when enabled) and copied body. If copying the log from
the AI Agent fails (for example, missing session, no transcript found, CLI failure,
or empty messages), `mm` automatically falls back to path-based commit message
generation instead of aborting with an error. In `--verbose` mode, the failure and
fallback are reported on stderr. It still uses the normal change selection,
`--add`, `--staged`, `--edit`, `--no-verify`, and `--print` behavior.

## commitlint configuration

`mm` searches upward from the current directory, stopping at `$HOME`, and reads
the same commitlint file names as cosmiconfig, including `package.json` and
`package.yaml`. JSON and YAML are parsed natively. JavaScript and TypeScript
configs are evaluated with `node`; `extends` presets additionally require
`@commitlint/load` in the project.

Applied rules are the ones that affect the generated header: type and scope
enums/case, type/scope presence, type/header/subject length, and trailing full
stops. Rules for bodies, footers, and trailers are not applied to generated
messages because `mm` writes only a one-line header. In chat modes (`--chat`, `--agy`, `--claude`, `--codex`, and `--devin`), `mm` applies header shaping rules (type prefix, casing, length, and trailing full stop removal) to the subject line, while copying the assistant body intact.

Use an explicit config with:

```sh
mm --config path/to/commitlint.config.js --print
```

## Examples

```sh
mm -p # preview without committing
mm -a # include untracked files, then commit
mm -s # commit only what is already staged
mm -e # review the message in an editor
mm --no-verify # bypass Git hooks
mm --chat # copy the current OpenCode or Antigravity chat
mm --agy --print # preview the last Antigravity conversation log
mm --agy-session conv-123 # commit with a specific Antigravity conversation
mm --devin --print # preview the last Devin / Cascade conversation log
mm --devin-session sess-123 # commit with a specific Devin session
mm --chat-session ses_123 --print # preview another OpenCode session
mm -m 'JIRA-1 fix: keep this' # keep an existing prefix/description
mm --verbose --print # explain what mm is doing on stderr
mm --completions fish > ~/.config/fish/completions/mm.fish # install fish completions
```

## Output and exit status

- The generated message goes to stdout.
- Errors go to stderr as `Error: ...`.
- Verbose diagnostics go to stderr with an `[mm]` prefix.
- Exit status is `0` on success (including `--help`, `--version`,
  `--completions`, and `--print`) and `1` on failure.

## Troubleshooting

`Unable to generate message as no changes files can be seen`
Nothing is staged and there are no tracked changes. Stage files with `git add`,
or use `mm --add` to include untracked files.

`not a git repository`
Run `mm` inside a Git work tree.

A JavaScript/TypeScript config is ignored or warns about `extends`
Install Node.js, and install `@commitlint/load` in the project that owns the
config if you use presets.

`mm.json: conventionalCommits must be a boolean`
The setting must be `true` or `false`. Fix the value in `mm.json`; an invalid
config stops `mm` before it stages or commits anything.

`no OpenCode session selected`
`--chat` needs `OPENCODE_SESSION_ID` from a running OpenCode session. Set it, or
pass `--chat-session <ID>`.

`could not run opencode session export`
Check that the `opencode` CLI is installed and on `PATH`, and that the session ID
exists.

`has no assistant text or completed compaction summary to copy`
The session has no assistant reply or completed compaction yet. Send a prompt
and wait for the answer, then rerun `mm --chat`.

`mm: command not found`
Check that `~/.cargo/bin` is in `PATH`, then start a new zsh session or run
`rehash`.

An old `ac` command still exists
This tool was renamed to `mm`. Reinstall with the command above; any previously
installed `ac` binary is separate and must be removed manually.
