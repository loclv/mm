//! Git integration.
//!
//! Only the commands which the original CLI used are wrapped here. Output is
//! parsed from Git's machine-friendly `--name-status -z` format so that paths
//! containing spaces, quotes or unicode are handled without escaping.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A file change reported by `git diff-index`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Status letter, e.g. `M`, `A`, `D`, `R` or `C`.
    pub x: char,
    /// Second status letter. `diff-index` does not report one, so this is
    /// always a space.
    pub y: char,
    /// Original path, or the only path for changes which are not renames.
    pub from: String,
    /// Destination path for renames and copies, otherwise empty.
    pub to: String,
}

impl FileChange {
    /// Build a change, using an unmodified (` `) second status.
    pub fn new(x: char, from: &str, to: &str) -> Self {
        Self {
            x,
            y: ' ',
            from: from.to_string(),
            to: to.to_string(),
        }
    }
}

/// Changes found in a repository.
#[derive(Debug, Clone)]
pub struct Changes {
    pub files: Vec<FileChange>,
    /// True when the changes came from the index rather than the worktree.
    pub staged: bool,
}

/// Options for creating a commit.
#[derive(Debug, Clone)]
pub struct CommitOptions<'a> {
    pub message: &'a str,
    /// Pass `.` as a pathspec so tracked, unstaged changes are included.
    pub pathspec: bool,
    /// Open the editor before committing.
    pub edit: bool,
    /// Skip Git hooks.
    pub no_verify: bool,
}

/// A Git repository, identified by its top-level directory.
#[derive(Debug)]
pub struct Git {
    root: PathBuf,
    verbose: bool,
}

impl Git {
    /// Find the repository which contains the current directory.
    pub fn discover(verbose: bool) -> Result<Self, String> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|error| format!("failed to run git: {error}"))?;

        if !output.status.success() {
            let detail = stderr_text(&output);
            return Err(if detail.is_empty() {
                "not a git repository".to_string()
            } else {
                format!("not a git repository: {detail}")
            });
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();

        Ok(Self {
            root: PathBuf::from(root),
            verbose,
        })
    }

    /// Repository root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stage every change, including untracked files.
    pub fn stage_all(&self) -> Result<(), String> {
        self.debug("staging all changes with `git add -A`");

        let output = self.run(&["add", "-A"])?;

        if !output.status.success() {
            return Err(format!(
                "`git add -A` failed: {}",
                stderr_text(&output).trim()
            ));
        }

        Ok(())
    }

    /// Find changed files, preferring the index over the worktree.
    ///
    /// Mirrors the original behaviour: if anything is staged, only staged
    /// changes are used. Otherwise all tracked changes are used. Untracked files
    /// are never included unless [`Git::stage_all`] was called first.
    pub fn changes(&self, staged_only: bool) -> Result<Changes, String> {
        let staged = self.diff_index(true)?;

        if !staged.is_empty() {
            self.debug(&format!("found {} staged change(s)", staged.len()));

            return Ok(Changes {
                files: staged,
                staged: true,
            });
        }

        if staged_only {
            return Ok(Changes {
                files: Vec::new(),
                staged: true,
            });
        }

        self.debug("staging area is empty, using tracked worktree changes");

        let files = self.diff_index(false)?;

        Ok(Changes {
            files,
            staged: false,
        })
    }

    /// Run `git diff-index` and parse the result.
    fn diff_index(&self, cached: bool) -> Result<Vec<FileChange>, String> {
        let base = self.head_or_empty_tree()?;

        let mut args: Vec<&str> = vec![
            "-c",
            "core.quotePath=false",
            "diff-index",
            "--name-status",
            "--find-renames",
            "--find-copies",
            "--no-color",
            "-z",
        ];

        if cached {
            args.push("--cached");
        }
        args.push(base.as_str());

        self.debug(&format!("running `git {}`", args.join(" ")));

        let output = self.run(&args)?;

        if !output.status.success() {
            return Err(format!(
                "`git diff-index` failed: {}",
                stderr_text(&output).trim()
            ));
        }

        parse_name_status_z(&output.stdout)
    }

    /// Use `HEAD`, or the empty tree when the repository has no commits yet.
    fn head_or_empty_tree(&self) -> Result<String, String> {
        let output = self.run(&["rev-parse", "--verify", "--quiet", "HEAD"])?;

        if output.status.success() {
            return Ok("HEAD".to_string());
        }

        let output = Command::new("git")
            .args(["hash-object", "-t", "tree", "--stdin"])
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("failed to run git: {error}"))?;

        if !output.status.success() {
            return Err(format!(
                "could not find a base revision: {}",
                stderr_text(&output).trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Commit with the generated message.
    ///
    /// The commit is printed to the terminal directly so that hooks,
    /// GPG signing and editor prompts behave exactly as with `git commit`.
    pub fn commit(&self, options: &CommitOptions<'_>) -> Result<(), String> {
        let mut command = Command::new("git");

        command
            .current_dir(&self.root)
            .arg("commit")
            .arg("-m")
            .arg(options.message);

        if options.edit {
            command.arg("--edit");
        }
        if options.no_verify {
            command.arg("--no-verify");
        }
        if options.pathspec {
            // Equivalent to the original `git commit -m MSG .` fallback.
            command.arg(".");
        }

        self.debug("running `git commit`");

        let status = command
            .status()
            .map_err(|error| format!("failed to run git: {error}"))?;

        if !status.success() {
            return Err(format!("git commit failed ({status})"));
        }

        Ok(())
    }

    /// Run a Git command in the repository root.
    fn run(&self, args: &[&str]) -> Result<Output, String> {
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .map_err(|error| format!("failed to run git: {error}"))
    }

    /// Print a message when verbose mode is enabled.
    fn debug(&self, message: &str) {
        if self.verbose {
            eprintln!("[ac] {message}");
        }
    }
}

/// Extract stderr as lossy UTF-8.
fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Parse output of `git diff-index --name-status -z`.
///
/// Records are NUL separated. A normal record is `STATUS\0path\0`; a rename or
/// copy is `STATUS\0old\0new\0`.
pub fn parse_name_status_z(bytes: &[u8]) -> Result<Vec<FileChange>, String> {
    let tokens: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut changes = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        if tokens[index].is_empty() {
            index += 1;
            continue;
        }

        let status = String::from_utf8_lossy(tokens[index]);
        let x = status
            .chars()
            .next()
            .ok_or_else(|| "empty status in diff output".to_string())?;

        index += 1;

        let from = token_text(tokens.get(index), x, "source path")?;
        index += 1;

        let to = if x == 'R' || x == 'C' {
            let to = token_text(tokens.get(index), x, "destination path")?;
            index += 1;
            to
        } else {
            String::new()
        };

        changes.push(FileChange::new(x, &from, &to));
    }

    Ok(changes)
}

/// Read a NUL-separated token as text.
fn token_text(token: Option<&&[u8]>, status: char, label: &str) -> Result<String, String> {
    let token = token
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("missing {label} for status `{status}`"))?;

    Ok(String::from_utf8_lossy(token).to_string())
}

/// Parse newline-separated `git diff-index --name-status` text.
///
/// This exists for hooks and scripts which already have the text and do not
/// want the tool to run Git again. Blank lines are ignored.
pub fn parse_name_status_text(text: &str) -> Result<Vec<FileChange>, String> {
    let mut changes = Vec::new();

    for line in text.lines() {
        let line = line.trim_end_matches('\r');

        if line.trim().is_empty() {
            continue;
        }

        let mut fields = line.split('\t');
        let status = fields
            .next()
            .ok_or_else(|| format!("invalid diff line: {line}"))?;
        let x = status
            .chars()
            .next()
            .ok_or_else(|| format!("invalid diff line: {line}"))?;

        let from = fields
            .next()
            .ok_or_else(|| format!("missing source path in: {line}"))?;

        let to = if x == 'R' || x == 'C' {
            fields
                .next()
                .ok_or_else(|| format!("missing destination path in: {line}"))?
        } else {
            ""
        };

        changes.push(FileChange::new(x, from, to));
    }

    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nul_separated_changes() {
        let input = b"M\0src/main.rs\0A\0src/new.rs\0R100\0src/old.rs\0src/new.rs\0";
        let changes = parse_name_status_z(input).unwrap();

        assert_eq!(
            changes,
            vec![
                FileChange {
                    x: 'M',
                    y: ' ',
                    from: "src/main.rs".to_string(),
                    to: String::new(),
                },
                FileChange {
                    x: 'A',
                    y: ' ',
                    from: "src/new.rs".to_string(),
                    to: String::new(),
                },
                FileChange {
                    x: 'R',
                    y: ' ',
                    from: "src/old.rs".to_string(),
                    to: "src/new.rs".to_string(),
                },
            ]
        );
    }

    #[test]
    fn keeps_spaces_and_unicode_in_paths() {
        let input = "M\0docs/über file.md\0C75\0a b/c.txt\0d/e f.txt\0".as_bytes();
        let changes = parse_name_status_z(input).unwrap();

        assert_eq!(changes[0].from, "docs/über file.md");
        assert_eq!(changes[1].from, "a b/c.txt");
        assert_eq!(changes[1].to, "d/e f.txt");
    }

    #[test]
    fn ignores_empty_input() {
        assert!(parse_name_status_z(b"").unwrap().is_empty());
        assert!(parse_name_status_text("").unwrap().is_empty());
    }

    #[test]
    fn fails_on_truncated_input() {
        assert!(parse_name_status_z(b"R100\0only-old\0").is_err());
        assert!(parse_name_status_text("M").is_err());
    }

    #[test]
    fn parses_text_output() {
        let changes =
            parse_name_status_text("M\tsrc/main.rs\n\nA\tsrc/new.rs\nR100\tsrc/a.rs\tsrc/b.rs\n")
                .unwrap();

        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].from, "src/main.rs");
        assert_eq!(changes[2].x, 'R');
        assert_eq!(changes[2].to, "src/b.rs");
    }
}
