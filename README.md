# mm - Rust CLI

`mm` is a Rust rewrite of the Auto Commit Message CLI logic. It generates a
Conventional Commit message from Git changes and commits in one step:

```sh
mm
```

No AI. The message is built from the paths and change types that Git reports,
using the same rules as the original TypeScript `gacm` CLI. If the project has a
commitlint configuration, its type, scope and length rules are applied too.

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
mm -m 'JIRA-1: fix' # Keep an existing prefix/description
```

Run `mm --help` for the full option list. See [docs/usage.md](docs/usage.md)
for macOS/zsh installation, the complete command reference, and troubleshooting.

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
are not applied, because `mm` does not write those parts. An explicit
`--message` value is treated as an override and its prefix is kept.

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
- Two to four files: names are listed, e.g. `update main.rs and lib.rs`
- Five or more files: counts are used, e.g. `update 5 files`
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
| `commitlint.rs` | New: finds and loads the project's commitlint configuration |
| `policy.rs` | New: applies commitlint rules to a generated message |

## Differences from `gacm`

The selection of changes and the message rules match `gacm`. These parts differ
on purpose:

- The commit editor is off by default; pass `--edit` to open it. `-n` and
  `--no-edit` are accepted so existing scripts keep working.
- `--print` (`--dry-run`) prints the message and never touches the repository.
- Paths are read with `git diff-index --name-status -z`, so spaces, quotes and
  unicode in file names do not need escaping.
- A repository without commits yet is supported by diffing against the empty
  tree.

## License

MIT
