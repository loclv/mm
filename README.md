# mm - Rust CLI

`mm` is a Rust rewrite of the [MichaelCurrin/auto-commit-msg](https://github.com/MichaelCurrin/auto-commit-msg) logic. It generates a
Conventional Commit message from Git changes and commits in one step:

```sh
mm
```

By default there is no AI or network call. The message is built from the paths
and change types that Git reports.
If the project has a commitlint configuration, its type,
scope and length rules are applied too. Inside OpenCode, `--chat` can copy the
current session title and last assistant message instead; see
[Using an OpenCode chat](#using-an-opencode-chat).

## Install

From the repository root:

```sh
cargo install --path .
```

The binary is called `mm`. To build without installing:

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
mm --chat # Copy the current OpenCode chat title and last message
mm -m 'JIRA-1: fix' # Keep an existing prefix/description (or override a chat subject)
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

### Using an OpenCode chat

OpenCode sets `OPENCODE_SESSION_ID` for the current chat. Pass `--chat` to copy
that session into the commit message:

```sh
mm --chat --print # preview the message
mm --chat # commit with it
```

- The subject is the session title.
- If the session has no title yet, `mm` uses the first line of the latest user
  prompt, truncated to 72 characters.
- The body is the last assistant message, matching OpenCode's `/copy` command.
  When the last message is a completed compaction, its summary is used. Tool
  calls and reasoning are omitted.
- `--chat-session <ID>` selects another session and implies `--chat`.
- `--message` overrides the subject; the copied message remains the body.
- `--chat` skips path-based generation and commitlint message shaping. It
  requires the `opencode` CLI, an existing local session, and an assistant
  message with text.

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
`--chat` copies its subject and body from the session and bypasses these
generation rules.

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
- Two to four files: names are listed, e.g. `update src/main.rs and src/lib.rs`; mixed
  actions fall back to counts
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
