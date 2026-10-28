//! Command-line interface for `mm`.
//!
//! `mm` looks at staged changes (falling back to tracked worktree changes),
//! generates a Conventional Commit message, and commits. Use `--print` to only
//! show the message.

use std::path::{Path, PathBuf};

use crate::chat;
use crate::commitlint;
use crate::completions::Shell;
use crate::config;
use crate::constants::NO_LINES_MSG;
use crate::generate::{MessageFormat, generate_msg_with_policy};
use crate::git::{CommitOptions, Git};
use crate::policy::Policy;

/// What the user asked for on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Completions(Shell),
    Run(Options),
}

/// Options for a normal run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Stage everything first, including untracked files.
    pub add: bool,
    /// Open the editor before committing.
    pub edit: bool,
    /// Print the message and do not commit.
    pub print_only: bool,
    /// Existing message to merge with the generated one, or to use as the chat
    /// subject.
    pub message: Option<String>,
    /// Copy the message from an OpenCode or Antigravity chat session.
    pub chat: bool,
    /// Force copying from Antigravity.
    pub antigravity: bool,
    /// Force copying from Claude Code.
    pub claude: bool,
    /// Force copying from Codex.
    pub codex: bool,
    /// Force copying from Devin / Codeium.
    pub devin: bool,
    /// Chat session or conversation ID to copy; implies `chat`.
    pub chat_session: Option<String>,
    /// Explicit commitlint configuration file.
    pub config: Option<PathBuf>,
    /// Never fall back to unstaged changes.
    pub staged_only: bool,
    /// Pass `--no-verify` to Git.
    pub no_verify: bool,
    /// Print diagnostics to stderr.
    pub verbose: bool,
}

/// Help text shown by `mm --help`.
pub const HELP: &str = "\
mm - commit with an automatically generated message

Usage:
  mm [options]

By default, staged changes are used. If nothing is staged, tracked changes in
the working tree are committed instead (like `git commit .`). Untracked files
are ignored unless you pass --add.

A commitlint configuration is detected automatically (JSON, YAML, JavaScript,
TypeScript, or a `commitlint` field in package.json/package.yaml) and its type,
scope and length rules are applied to the generated message.

Messages follow Conventional Commits by default. Add `mm.json` with
{\"conventionalCommits\": false} to generate plain descriptions instead, or
{\"autoAdd\": true} to automatically stage all changes when none are staged.

Pass --chat to copy the title and last assistant message from the current
OpenCode, Antigravity, Claude Code, Codex, or Devin session instead; chat messages are not
reshaped by commitlint. If copying from chat fails, mm automatically falls back
to auto commit message generation. Use --chat-session <ID> to select a different
session, --agy to copy from Antigravity, --claude to copy from Claude Code, --codex
to copy from Codex, or --devin to copy from Devin / Codeium.

Options:
      --completions <SHELL> # Print a completion script for bash, zsh, or fish
  -a, --add # Stage all changes, including untracked files, first
  -e, --edit # Open the commit message in an editor before committing
  -p, --print # Print the generated message and exit without committing
  -d, --dry-run # Alias for --print
  -m, --message <MSG> # Merge the generated description (or override a chat subject)
      --chat # Copy the chat title and last message (OpenCode, Antigravity, Claude Code, Codex, or Devin)
      --chat-session <ID> # Use a specific session ID or transcript path (implies --chat)
      --agy, --antigravity # Copy the Antigravity last log commit message (implies --chat)
      --agy-session <ID> # Use a specific Antigravity conversation (implies --agy)
      --claude # Copy the Claude Code last log commit message (implies --chat)
      --claude-session <ID> # Use a specific Claude Code session (implies --claude)
      --codex # Copy the Codex last log commit message (implies --chat)
      --codex-session <ID> # Use a specific Codex session (implies --codex)
      --devin, --codeium # Copy the Devin / Codeium last log commit message (implies --chat)
      --devin-session <ID> # Use a specific Devin / Codeium session (implies --devin)
  -c, --config <FILE> # Use this commitlint configuration instead of searching
  -s, --staged # Only use staged changes; never fall back to unstaged
      --no-verify # Bypass Git commit hooks
      --verbose # Show what the tool is doing on stderr
  -n, --no-edit # Accepted for compatibility; editing is off by default
  -h, --help # Show this help and exit
  -v, -V, --version # Show the version and exit

Examples:
  mm # Generate a message and commit
  mm --print # Show the message without committing
  mm --add # Include new files, then commit
  mm --chat --print # Preview the message copied from the current chat
  mm --agy --print # Preview the message copied from Antigravity's last log
  mm --claude --print # Preview the message copied from Claude Code's last log
  mm --codex --print # Preview the message copied from Codex's last log
  mm --devin --print # Preview the message copied from Devin's last log
  mm -m 'JIRA-1: fix' # Keep the existing prefix/description
  mm --completions zsh > ~/.zfunc/_mm # Install zsh completions
";

/// Parse arguments into a command.
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut options = Options::default();
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();

        match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "-v" | "-V" | "--version" => return Ok(Command::Version),
            "--completions" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;

                return parse_shell(value).map(Command::Completions);
            }
            "-a" | "--add" | "--all" => options.add = true,
            "-e" | "--edit" => options.edit = true,
            "-n" | "--no-edit" => options.edit = false,
            "-p" | "--print" | "-d" | "--dry-run" | "--no-commit" => {
                options.print_only = true;
            }
            "-c" | "--config" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.config = Some(PathBuf::from(value));
            }
            "-s" | "--staged" | "--staged-only" => options.staged_only = true,
            "--chat" | "--from-chat" => options.chat = true,
            "--agy" | "--antigravity" => {
                options.chat = true;
                options.antigravity = true;
            }
            "--agy-session" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.chat = true;
                options.antigravity = true;
                options.chat_session = Some(value.clone());
            }
            "--claude" => {
                options.chat = true;
                options.claude = true;
            }
            "--claude-session" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.chat = true;
                options.claude = true;
                options.chat_session = Some(value.clone());
            }
            "--codex" => {
                options.chat = true;
                options.codex = true;
            }
            "--codex-session" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.chat = true;
                options.codex = true;
                options.chat_session = Some(value.clone());
            }
            "--devin" | "--codeium" => {
                options.chat = true;
                options.devin = true;
            }
            "--devin-session" | "--codeium-session" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.chat = true;
                options.devin = true;
                options.chat_session = Some(value.clone());
            }
            "--chat-session" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.chat = true;
                options.chat_session = Some(value.clone());
            }
            "--no-verify" => options.no_verify = true,
            "--verbose" => options.verbose = true,
            "-m" | "--message" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.message = Some(value.clone());
            }
            _ if arg.starts_with("--completions=") => {
                return parse_shell(&arg["--completions=".len()..]).map(Command::Completions);
            }
            _ if arg.starts_with("--message=") => {
                options.message = Some(arg["--message=".len()..].to_string());
            }
            _ if arg.starts_with("--config=") => {
                options.config = Some(PathBuf::from(&arg["--config=".len()..]));
            }
            _ if arg.starts_with("--agy-session=") => {
                options.chat = true;
                options.antigravity = true;
                options.chat_session = Some(arg["--agy-session=".len()..].to_string());
            }
            _ if arg.starts_with("--claude-session=") => {
                options.chat = true;
                options.claude = true;
                options.chat_session = Some(arg["--claude-session=".len()..].to_string());
            }
            _ if arg.starts_with("--codex-session=") => {
                options.chat = true;
                options.codex = true;
                options.chat_session = Some(arg["--codex-session=".len()..].to_string());
            }
            _ if arg.starts_with("--devin-session=") => {
                options.chat = true;
                options.devin = true;
                options.chat_session = Some(arg["--devin-session=".len()..].to_string());
            }
            _ if arg.starts_with("--codeium-session=") => {
                options.chat = true;
                options.devin = true;
                options.chat_session = Some(arg["--codeium-session=".len()..].to_string());
            }
            _ if arg.starts_with("--chat-session=") => {
                options.chat = true;
                options.chat_session = Some(arg["--chat-session=".len()..].to_string());
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown option `{arg}`\n\n{HELP}"));
            }
            _ => {
                return Err(format!("unexpected argument `{arg}`\n\n{HELP}"));
            }
        }

        index += 1;
    }

    Ok(Command::Run(options))
}

/// Parse the shell name accepted by `--completions`.
fn parse_shell(value: &str) -> Result<Shell, String> {
    Shell::parse(value).ok_or_else(|| {
        format!(
            "unsupported shell `{value}` (expected {})",
            Shell::NAMES.join(", ")
        )
    })
}

/// Run the CLI and print any requested output.
///
/// Errors are returned as messages for `main` to print. This keeps the function
/// testable and avoids calling `process::exit` deep in the logic.
pub fn run(args: &[String]) -> Result<(), String> {
    match parse_args(args)? {
        Command::Help => {
            print!("{HELP}");
            Ok(())
        }
        Command::Version => {
            println!("mm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Completions(shell) => {
            print!("{}", shell.script());
            Ok(())
        }
        Command::Run(options) => execute(&options),
    }
}

/// Generate the message and optionally commit it.
pub fn execute(options: &Options) -> Result<(), String> {
    let git = Git::discover(options.verbose)?;

    if options.verbose {
        eprintln!("[mm] repository: {}", git.root().display());
    }

    let directory = std::env::current_dir()
        .map_err(|error| format!("could not determine the current directory: {error}"))?;

    let settings = if options.chat {
        config::load(&directory).unwrap_or_default()
    } else {
        config::load(&directory)?
    };

    if options.add && !options.print_only {
        git.stage_all()?;
    }

    let mut changes = git.changes(options.staged_only)?;

    if !options.add
        && settings.auto_add
        && !options.staged_only
        && !options.print_only
        && !changes.staged
    {
        if options.verbose {
            eprintln!("[mm] auto add: no staged changes found; staging all changes");
        }
        git.stage_all()?;
        changes = git.changes(options.staged_only)?;
    }

    if changes.files.is_empty() {
        return Err(NO_LINES_MSG.to_string());
    }

    let message = if options.chat {
        let chat_result = chat::load(
            options.chat_session.as_deref(),
            Some(git.root()),
            options.antigravity,
            options.claude,
            options.codex,
            options.devin,
            options.verbose,
        );

        match chat_result {
            Ok(chat) => {
                let policy = load_policy(options, &directory).unwrap_or_default();
                let initial_subject = options
                    .message
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .unwrap_or(&chat.subject);

                let formatted_header = if settings.conventional_commits {
                    crate::generate::format_chat_header(initial_subject, &changes.files, &policy)
                } else {
                    initial_subject.to_string()
                };

                let body = chat::sanitize_chat_body(&chat.body, Some(git.root()));
                let msg = if body.is_empty() {
                    formatted_header
                } else {
                    format!("{formatted_header}\n\n{body}")
                };
                let msg = chat::sanitize_current_path(&msg, Some(git.root()));

                if msg.is_empty() {
                    if options.verbose {
                        eprintln!(
                            "[mm] chat: message from session {} is empty; falling back to auto commit message",
                            chat.session_id
                        );
                    }
                    generate_auto_message(options, &directory, &changes.files, &settings)?
                } else {
                    if options.verbose {
                        eprintln!(
                            "[mm] chat: session {} ({})",
                            chat.session_id,
                            match chat.subject_source {
                                chat::SubjectSource::Title => "using the session title",
                                chat::SubjectSource::LatestPrompt => {
                                    "no session title; using the latest user prompt"
                                }
                                chat::SubjectSource::LastMessage => {
                                    "no session title or user prompt; using the copied message"
                                }
                            }
                        );
                    }
                    msg
                }
            }
            Err(error) => {
                if options.verbose {
                    eprintln!(
                        "[mm] chat: failed to copy log from AI Agent ({error}); falling back to auto commit message"
                    );
                }
                generate_auto_message(options, &directory, &changes.files, &settings)?
            }
        }
    } else {
        generate_auto_message(options, &directory, &changes.files, &settings)?
    };

    if message.is_empty() {
        return Err(NO_LINES_MSG.to_string());
    }

    if options.print_only {
        println!("{message}");
        return Ok(());
    }

    if options.verbose {
        eprintln!("[mm] message: {message}");
    }

    git.commit(&CommitOptions {
        message: &message,
        pathspec: !changes.staged,
        edit: options.edit,
        no_verify: options.no_verify,
    })
}

/// Generate the deterministic, path-based commit message.
fn generate_auto_message(
    options: &Options,
    directory: &Path,
    files: &[crate::git::FileChange],
    settings: &config::Config,
) -> Result<String, String> {
    if options.verbose {
        match &settings.source {
            Some(source) => eprintln!(
                "[mm] config: {} (conventional commits: {})",
                source.display(),
                if settings.conventional_commits {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
            None => eprintln!("[mm] config: none (conventional commits: enabled)"),
        }
    }

    let policy = load_policy(options, directory)?;
    let format = if settings.conventional_commits {
        MessageFormat::Conventional
    } else {
        MessageFormat::Plain
    };

    generate_msg_with_policy(files, options.message.as_deref(), &policy, format)
}

/// Read the project's commitlint configuration, reporting any warnings.
fn load_policy(options: &Options, directory: &Path) -> Result<Policy, String> {
    let config = commitlint::load(directory, options.config.as_deref())?;

    let Some(config) = config else {
        return Ok(Policy::default());
    };

    if options.verbose {
        eprintln!(
            "[mm] commitlint config: {} (loaded via {})",
            config.source.display(),
            config.via
        );
        if config.policy.is_empty() {
            eprintln!("[mm] no generation rules found in commitlint config");
        }
    }

    for warning in &config.warnings {
        eprintln!("warning: {warning}");
    }

    Ok(config.policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_help_and_version() {
        assert_eq!(parse_args(&args(&["-h"])).unwrap(), Command::Help);
        assert_eq!(parse_args(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse_args(&args(&["-v"])).unwrap(), Command::Version);
        assert_eq!(parse_args(&args(&["-V"])).unwrap(), Command::Version);
        assert_eq!(parse_args(&args(&["--version"])).unwrap(), Command::Version);
    }

    #[test]
    fn parses_no_arguments_as_default_run() {
        assert_eq!(parse_args(&[]).unwrap(), Command::Run(Options::default()));
    }

    #[test]
    fn parses_flags() {
        let command = parse_args(&args(&[
            "--add",
            "--edit",
            "--print",
            "--staged",
            "--no-verify",
            "--verbose",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::Run(Options {
                add: true,
                edit: true,
                print_only: true,
                message: None,
                chat: false,
                antigravity: false,
                claude: false,
                codex: false,
                devin: false,
                chat_session: None,
                config: None,
                staged_only: true,
                no_verify: true,
                verbose: true,
            })
        );
    }

    #[test]
    fn parses_message_values() {
        let with_space = parse_args(&args(&["-m", "fix: words"])).unwrap();
        let with_equals = parse_args(&args(&["--message=fix: words"])).unwrap();

        assert_eq!(with_space, with_equals);
        assert_eq!(
            with_space,
            Command::Run(Options {
                message: Some("fix: words".to_string()),
                ..Options::default()
            })
        );
    }

    #[test]
    fn rejects_missing_message_value() {
        assert!(parse_args(&args(&["--message"])).is_err());
        assert!(parse_args(&args(&["--config"])).is_err());
    }

    #[test]
    fn parses_chat_flags() {
        assert_eq!(
            parse_args(&args(&["--chat"])).unwrap(),
            Command::Run(Options {
                chat: true,
                ..Options::default()
            })
        );

        assert_eq!(
            parse_args(&args(&["--agy"])).unwrap(),
            Command::Run(Options {
                chat: true,
                antigravity: true,
                ..Options::default()
            })
        );

        assert_eq!(
            parse_args(&args(&["--antigravity"])).unwrap(),
            Command::Run(Options {
                chat: true,
                antigravity: true,
                ..Options::default()
            })
        );

        let with_space = parse_args(&args(&["--chat-session", "ses_test"])).unwrap();
        let with_equals = parse_args(&args(&["--chat-session=ses_test"])).unwrap();

        assert_eq!(with_space, with_equals);
        assert_eq!(
            with_space,
            Command::Run(Options {
                chat: true,
                chat_session: Some("ses_test".to_string()),
                ..Options::default()
            })
        );

        let agy_space = parse_args(&args(&["--agy-session", "conv_test"])).unwrap();
        let agy_equals = parse_args(&args(&["--agy-session=conv_test"])).unwrap();

        assert_eq!(agy_space, agy_equals);
        assert_eq!(
            agy_space,
            Command::Run(Options {
                chat: true,
                antigravity: true,
                chat_session: Some("conv_test".to_string()),
                ..Options::default()
            })
        );

        assert_eq!(
            parse_args(&args(&["--claude"])).unwrap(),
            Command::Run(Options {
                chat: true,
                claude: true,
                ..Options::default()
            })
        );

        let claude_space = parse_args(&args(&["--claude-session", "claude_test"])).unwrap();
        let claude_equals = parse_args(&args(&["--claude-session=claude_test"])).unwrap();

        assert_eq!(claude_space, claude_equals);
        assert_eq!(
            claude_space,
            Command::Run(Options {
                chat: true,
                claude: true,
                chat_session: Some("claude_test".to_string()),
                ..Options::default()
            })
        );

        assert_eq!(
            parse_args(&args(&["--codex"])).unwrap(),
            Command::Run(Options {
                chat: true,
                codex: true,
                ..Options::default()
            })
        );

        let codex_space = parse_args(&args(&["--codex-session", "codex_test"])).unwrap();
        let codex_equals = parse_args(&args(&["--codex-session=codex_test"])).unwrap();

        assert_eq!(codex_space, codex_equals);
        assert_eq!(
            codex_space,
            Command::Run(Options {
                chat: true,
                codex: true,
                chat_session: Some("codex_test".to_string()),
                ..Options::default()
            })
        );

        assert_eq!(
            parse_args(&args(&["--devin"])).unwrap(),
            Command::Run(Options {
                chat: true,
                devin: true,
                ..Options::default()
            })
        );

        assert_eq!(
            parse_args(&args(&["--codeium"])).unwrap(),
            Command::Run(Options {
                chat: true,
                devin: true,
                ..Options::default()
            })
        );

        let devin_space = parse_args(&args(&["--devin-session", "devin_test"])).unwrap();
        let devin_equals = parse_args(&args(&["--devin-session=devin_test"])).unwrap();
        assert_eq!(devin_space, devin_equals);
        assert_eq!(
            devin_space,
            Command::Run(Options {
                chat: true,
                devin: true,
                chat_session: Some("devin_test".to_string()),
                ..Options::default()
            })
        );

        let codeium_equals = parse_args(&args(&["--codeium-session=codeium_test"])).unwrap();
        assert_eq!(
            codeium_equals,
            Command::Run(Options {
                chat: true,
                devin: true,
                chat_session: Some("codeium_test".to_string()),
                ..Options::default()
            })
        );
    }

    #[test]
    fn rejects_a_missing_chat_session() {
        assert!(parse_args(&args(&["--chat-session"])).is_err());
        assert!(parse_args(&args(&["--agy-session"])).is_err());
        assert!(parse_args(&args(&["--claude-session"])).is_err());
        assert!(parse_args(&args(&["--codex-session"])).is_err());
        assert!(parse_args(&args(&["--devin-session"])).is_err());
        assert!(parse_args(&args(&["--codeium-session"])).is_err());
    }

    #[test]
    fn parses_config_paths() {
        let with_space = parse_args(&args(&["--config", "commitlint.config.js"])).unwrap();
        let with_equals = parse_args(&args(&["--config=commitlint.config.js"])).unwrap();

        assert_eq!(with_space, with_equals);
        assert_eq!(
            with_space,
            Command::Run(Options {
                config: Some(PathBuf::from("commitlint.config.js")),
                ..Options::default()
            })
        );
    }

    #[test]
    fn rejects_unknown_arguments() {
        assert!(parse_args(&args(&["--nope"])).is_err());
        assert!(parse_args(&args(&["commit"])).is_err());
    }

    #[test]
    fn parses_completions() {
        let by_space = parse_args(&args(&["--completions", "bash"])).unwrap();
        let by_equals = parse_args(&args(&["--completions=bash"])).unwrap();
        let zsh = parse_args(&args(&["--completions", "zsh"])).unwrap();
        let fish = parse_args(&args(&["--completions=fish"])).unwrap();

        assert_eq!(by_space, Command::Completions(Shell::Bash));
        assert_eq!(by_equals, by_space);
        assert_eq!(zsh, Command::Completions(Shell::Zsh));
        assert_eq!(fish, Command::Completions(Shell::Fish));
    }

    #[test]
    fn rejects_unsupported_completion_shells() {
        assert!(parse_args(&args(&["--completions"])).is_err());

        let error = parse_args(&args(&["--completions", "nushell"])).unwrap_err();
        assert!(error.contains("unsupported shell `nushell`"), "{error}");
        assert!(error.contains("bash, zsh, fish"), "{error}");
    }

    #[test]
    fn no_edit_keeps_editing_off() {
        assert_eq!(
            parse_args(&args(&["-n"])).unwrap(),
            Command::Run(Options::default())
        );
    }
}
