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
| `-a`, `--add` | Stage all changes, including untracked files, then commit |
| `-e`, `--edit` | Open the generated message in Git's editor before committing |
| `-p`, `--print` | Print the message and exit; never commit |
| `-d`, `--dry-run` | Alias for `--print` |
| `-m`, `--message <MSG>` | Merge the generated description with an existing message, or override the chat subject |
| `--chat` | Copy the current OpenCode chat title and last message |
| `--chat-session <ID>` | Use a specific OpenCode session; implies `--chat` |
| `-c`, `--config <FILE>` | Use a specific commitlint config instead of searching |
| `-s`, `--staged` | Only use staged changes; never fall back to unstaged changes |
| `--no-verify` | Bypass Git commit hooks |
| `--verbose` | Print diagnostics to stderr |
| `-n`, `--no-edit` | Compatibility flag; editing is already off by default |
| `-h`, `--help` | Show help and exit |
| `-v`, `-V`, `--version` | Show the version and exit |

## How changes are selected

1. If anything is staged, only staged changes are used.
2. If nothing is staged, tracked working-tree changes are committed with
   `git commit -m MSG .`.
3. Untracked files are ignored unless `--add` is passed.

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
- Two to four files: names are listed when the actions match, for example
  `chore: update main.rs and lib.rs`; mixed actions fall back to counts.
- Five or more files: counts are used, for example `chore: update 5 files`.
- Types come from the path: docs, tests, CI, dependencies (`build(deps)`), build
  files, and configuration (`chore`).
- A new file with no other match is a `feat`.
- Renames, moves, and deletions are `chore`.
- When no type can be inferred, or the changed paths disagree, the type is
  `chore`.

An existing type prefix supplied with `--message` wins over the generated type;
an existing description is appended after the generated description.

### Disabling the convention with `mm.json`

Add `mm.json` to the project (or a parent directory up to `$HOME`) to disable the
Conventional Commits format:

```json
{
  "conventionalCommits": false
}
```

`mm` then generates plain descriptions such as `update README.md`; it does not
add a `chore` fallback or any `type:` prefix. The short alias `"conventional"`
is also accepted. An explicit `--message` prefix is still preserved, and
commitlint rules still apply.

`mm.json` is searched upward from the current directory, stopping at your home
directory, the same way commitlint configuration is found. `--config` is
unrelated: it overrides the commitlint configuration file.

## Using an OpenCode chat

OpenCode exports each session with its `opencode` CLI. When `mm` runs inside an
OpenCode session, `OPENCODE_SESSION_ID` identifies the current chat:

```sh
mm --chat --print # preview the copied message
mm --chat         # commit with it
```

The message is built like this:

- The subject is the OpenCode session title.
- If the title is missing or still a `New session ...` placeholder, `mm` uses
  the first line of the latest user prompt, truncated to 72 characters. Without
  a user prompt, the first line of the copied message is used.
- The body is the last assistant message, matching OpenCode's `/copy` command.
  Only `text` parts are copied; tool calls and reasoning are left out. If the
  last message is a completed compaction, its summary is copied instead.
- `--chat-session <ID>` selects another session and implies `--chat`.
- `--message` overrides the subject; the copied message remains the body.

`--chat` skips path-based generation, `mm.json` formatting, and commitlint
message shaping. It still uses the normal change selection, `--add`, `--staged`,
`--edit`, `--no-verify`, and `--print` behavior. The `opencode` CLI must be on
`PATH`, the session must exist locally, and the session must contain an
assistant message with text or a completed compaction summary.

## commitlint configuration

`mm` searches upward from the current directory, stopping at `$HOME`, and reads
the same commitlint file names as cosmiconfig, including `package.json` and
`package.yaml`. JSON and YAML are parsed natively. JavaScript and TypeScript
configs are evaluated with `node`; `extends` presets additionally require
`@commitlint/load` in the project.

Applied rules are the ones that affect the generated header: type and scope
enums/case, type/scope presence, type/header/subject length, and trailing full
stops. Rules for bodies, footers, and trailers are not applied to generated
messages because `mm` writes only a one-line header. `--chat` copies a
multi-line message and bypasses these generation rules.

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
mm --chat # copy the current OpenCode chat
mm --chat-session ses_123 --print # preview another session
mm -m 'JIRA-1 fix: keep this' # keep an existing prefix/description
mm --verbose --print # explain what mm is doing on stderr
```

## Output and exit status

- The generated message goes to stdout.
- Errors go to stderr as `Error: ...`.
- Verbose diagnostics go to stderr with an `[mm]` prefix.
- Exit status is `0` on success (including `--help`, `--version`, and
  `--print`) and `1` on failure.

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
