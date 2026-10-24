//! Command-line interface for `ac`.
//!
//! `ac` looks at staged changes (falling back to tracked worktree changes),
//! generates a Conventional Commit message, and commits. Use `--print` to only
//! show the message.

use std::path::PathBuf;

use crate::commitlint;
use crate::constants::NO_LINES_MSG;
use crate::generate::generate_msg_with_policy;
use crate::git::{CommitOptions, Git};
use crate::policy::Policy;

/// What the user asked for on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
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
    /// Existing message to merge with the generated one.
    pub message: Option<String>,
    /// Explicit commitlint configuration file.
    pub config: Option<PathBuf>,
    /// Never fall back to unstaged changes.
    pub staged_only: bool,
    /// Pass `--no-verify` to Git.
    pub no_verify: bool,
    /// Print diagnostics to stderr.
    pub verbose: bool,
}

/// Help text shown by `ac --help`.
pub const HELP: &str = "\
ac - commit with an automatically generated message

Usage:
  ac [options]

By default, staged changes are used. If nothing is staged, tracked changes in
the working tree are committed instead (like `git commit .`). Untracked files
are ignored unless you pass --add.

A commitlint configuration is detected automatically (JSON, YAML, JavaScript,
TypeScript, or a `commitlint` field in package.json/package.yaml) and its type,
scope and length rules are applied to the generated message.

Options:
  -a, --add              Stage all changes, including untracked files, first
  -e, --edit             Open the commit message in an editor before committing
  -p, --print            Print the generated message and exit without committing
  -d, --dry-run          Alias for --print
  -m, --message <MSG>    Merge the generated description with an existing message
  -c, --config <FILE>    Use this commitlint configuration instead of searching
  -s, --staged           Only use staged changes; never fall back to unstaged
      --no-verify        Bypass Git commit hooks
  -v, --verbose          Show what the tool is doing on stderr
  -n, --no-edit          Accepted for compatibility; editing is off by default
  -h, --help             Show this help and exit
  -V, --version          Show the version and exit

Examples:
  ac                     Generate a message and commit
  ac --print             Show the message without committing
  ac --add               Include new files, then commit
  ac -m 'JIRA-1: fix'    Keep the existing prefix/description
";

/// Parse arguments into a command.
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut options = Options::default();
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].as_str();

        match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
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
            "--no-verify" => options.no_verify = true,
            "-v" | "--verbose" => options.verbose = true,
            "-m" | "--message" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("`{arg}` requires a value"))?;
                options.message = Some(value.clone());
            }
            _ if arg.starts_with("--message=") => {
                options.message = Some(arg["--message=".len()..].to_string());
            }
            _ if arg.starts_with("--config=") => {
                options.config = Some(PathBuf::from(&arg["--config=".len()..]));
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
            println!("ac {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Run(options) => execute(&options),
    }
}

/// Generate the message and optionally commit it.
pub fn execute(options: &Options) -> Result<(), String> {
    let git = Git::discover(options.verbose)?;

    if options.verbose {
        eprintln!("[ac] repository: {}", git.root().display());
    }

    if options.add && !options.print_only {
        git.stage_all()?;
    }

    let changes = git.changes(options.staged_only)?;

    if changes.files.is_empty() {
        return Err(NO_LINES_MSG.to_string());
    }

    let policy = load_policy(options)?;

    let message = generate_msg_with_policy(&changes.files, options.message.as_deref(), &policy)?;

    if message.is_empty() {
        return Err(NO_LINES_MSG.to_string());
    }

    if options.print_only {
        println!("{message}");
        return Ok(());
    }

    if options.verbose {
        eprintln!("[ac] message: {message}");
    }

    git.commit(&CommitOptions {
        message: &message,
        pathspec: !changes.staged,
        edit: options.edit,
        no_verify: options.no_verify,
    })
}

/// Read the project's commitlint configuration, reporting any warnings.
fn load_policy(options: &Options) -> Result<Policy, String> {
    let directory = std::env::current_dir()
        .map_err(|error| format!("could not determine the current directory: {error}"))?;

    let config = commitlint::load(&directory, options.config.as_deref())?;

    let Some(config) = config else {
        return Ok(Policy::default());
    };

    if options.verbose {
        eprintln!(
            "[ac] commitlint config: {} (loaded via {})",
            config.source.display(),
            config.via
        );
        if config.policy.is_empty() {
            eprintln!("[ac] no generation rules found in commitlint config");
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
        assert_eq!(parse_args(&args(&["-V"])).unwrap(), Command::Version);
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
    fn no_edit_keeps_editing_off() {
        assert_eq!(
            parse_args(&args(&["-n"])).unwrap(),
            Command::Run(Options::default())
        );
    }
}
