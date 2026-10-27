//! Git integration.
//!
//! Only the commands which the original CLI used are wrapped here. Output is
//! parsed from Git's machine-friendly `--name-status -z` format so that paths
//! containing spaces, quotes or unicode are handled without escaping.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

/// A file change reported by `git diff-index`.
///
/// In zero-copy parsing mode, `from` and `to` are borrowed `Cow::Borrowed(&'a str)`
/// pointing directly into the input byte buffer (such as Git's stdout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange<'a> {
    /// Status letter, e.g. `M`, `A`, `D`, `R` or `C`.
    pub x: char,
    /// Second status letter. `diff-index` does not report one, so this is
    /// always a space.
    pub y: char,
    /// Original path, or the only path for changes which are not renames.
    pub from: Cow<'a, str>,
    /// Destination path for renames and copies, otherwise empty.
    pub to: Cow<'a, str>,
}

impl<'a> FileChange<'a> {
    /// Build a change with borrowed string references (zero-copy).
    #[inline]
    pub fn borrowed(x: char, from: &'a str, to: &'a str) -> Self {
        Self {
            x,
            y: ' ',
            from: Cow::Borrowed(from),
            to: Cow::Borrowed(to),
        }
    }

    /// Build a change, using an unmodified (` `) second status.
    /// Accepts any string types convertible into `Cow<'a, str>`.
    #[inline]
    pub fn new(x: char, from: impl Into<Cow<'a, str>>, to: impl Into<Cow<'a, str>>) -> Self {
        Self {
            x,
            y: ' ',
            from: from.into(),
            to: to.into(),
        }
    }

    /// Convert into an owned `FileChange<'static>` with guaranteed `'static` lifetime.
    pub fn into_owned(self) -> FileChange<'static> {
        FileChange {
            x: self.x,
            y: self.y,
            from: Cow::Owned(self.from.into_owned()),
            to: Cow::Owned(self.to.into_owned()),
        }
    }
}

/// Changes found in a repository.
#[derive(Debug, Clone)]
pub struct Changes<'a> {
    pub files: Vec<FileChange<'a>>,
    /// True when the changes came from the index rather than the worktree.
    pub staged: bool,
    /// Backing buffer for zero-copy parsed file paths.
    /// Kept alive in an `Arc<[u8]>` to ensure borrowed string slices remain valid
    /// at a stable heap address for the lifetime of this struct and any clones.
    _buffer: Option<Arc<[u8]>>,
}

impl<'a> Changes<'a> {
    /// Construct changes without an owned buffer (e.g. for synthetic or test changes).
    pub fn new(files: Vec<FileChange<'a>>, staged: bool) -> Self {
        Self {
            files,
            staged,
            _buffer: None,
        }
    }

    /// Construct `Changes` backed by an owned buffer with zero string copies.
    ///
    /// # Safety & Performance Justification
    ///
    /// The input buffer is stored in an `Arc<[u8]>`, which guarantees a stable,
    /// immutable heap address. Even if `Changes` is moved on the stack or cloned,
    /// the underlying memory location of the buffer is guaranteed not to move or be freed
    /// until all references are dropped.
    ///
    /// The slices in `files` are parsed directly from this heap buffer. Transmuting
    /// the lifetime to `'static` is safe because `_buffer` is owned by `Changes`
    /// and stays alive for the exact lifespan of `Changes`.
    pub fn from_buffer(buffer: Arc<[u8]>, staged: bool) -> Result<Changes<'static>, String> {
        let changes = parse_name_status_z(&buffer)?;

        // SAFETY:
        // 1. `buffer` is pinned on the heap via `Arc<[u8]>`. Its memory address is immutable
        //    and will not move when `Changes` is moved or cloned.
        // 2. The string slices in `changes` point directly into this heap buffer.
        // 3. `Changes` keeps `_buffer` alive for as long as `Changes` exists.
        // 4. `FileChange<'a>` and `FileChange<'static>` have identical layout and alignment.
        let static_files: Vec<FileChange<'static>> = unsafe { std::mem::transmute(changes) };

        Ok(Changes {
            files: static_files,
            staged,
            _buffer: Some(buffer),
        })
    }
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
    pub fn changes(&self, staged_only: bool) -> Result<Changes<'static>, String> {
        let staged = self.diff_index(true)?;

        if !staged.files.is_empty() {
            self.debug(&format!("found {} staged change(s)", staged.files.len()));

            return Ok(staged);
        }

        if staged_only {
            return Ok(Changes::new(Vec::new(), true));
        }

        self.debug("staging area is empty, using tracked worktree changes");

        self.diff_index(false)
    }

    /// Run `git diff-index` and parse the result with zero copies.
    fn diff_index(&self, cached: bool) -> Result<Changes<'static>, String> {
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

        let buffer = Arc::from(output.stdout.into_boxed_slice());
        Changes::from_buffer(buffer, cached)
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
            eprintln!("[mm] {message}");
        }
    }
}

/// Extract stderr as lossy UTF-8.
fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Convert a raw byte slice from Git output into a string slice with zero copies.
///
/// # Performance & Safety Justification
///
/// Git path outputs under `--name-status -z` with `core.quotePath=false` are raw
/// bytes. In typical repositories, file paths are 100% valid ASCII.
///
/// 1. We first perform an optimized ASCII check (`token.is_ascii()`).
/// 2. If all bytes are ASCII, they are guaranteed to be valid UTF-8 without needing
///    a multi-byte UTF-8 state-machine validation pass. We use
///    `unsafe { std::str::from_utf8_unchecked(token) }` to eliminate redundant
///    validation overhead entirely and return a borrowed slice.
/// 3. If non-ASCII bytes are present, we attempt standard `std::str::from_utf8(token)`
///    to return a zero-copy borrowed slice for valid UTF-8 Unicode filenames.
/// 4. As a resilient fallback for non-UTF8 filesystem encodings, we decode lossy
///    into an owned `Cow::Owned(String)`.
#[inline]
pub fn bytes_to_cow_str<'a>(token: &'a [u8]) -> Cow<'a, str> {
    if token.is_ascii() {
        // SAFETY: `token.is_ascii()` verifies that every byte in the slice is <= 127.
        // The ASCII encoding is an exact 1-byte subset of UTF-8, making any ASCII byte
        // sequence strictly valid UTF-8. Using `from_utf8_unchecked` avoids the CPU
        // overhead of scanning the slice a second time through the multi-byte UTF-8
        // validation state machine.
        let s = unsafe { std::str::from_utf8_unchecked(token) };
        Cow::Borrowed(s)
    } else {
        match std::str::from_utf8(token) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => Cow::Owned(String::from_utf8_lossy(token).into_owned()),
        }
    }
}

/// Parse output of `git diff-index --name-status -z` with zero copies.
///
/// Records are NUL separated. A normal record is `STATUS\0path\0`; a rename or
/// copy is `STATUS\0old\0new\0`.
///
/// Every path string is borrowed directly from the input byte slice without heap allocation.
pub fn parse_name_status_z<'a>(bytes: &'a [u8]) -> Result<Vec<FileChange<'a>>, String> {
    let mut changes = Vec::new();
    let mut iter = bytes.split(|&b| b == 0);

    while let Some(status_bytes) = iter.next() {
        if status_bytes.is_empty() {
            continue;
        }

        // Fast extraction of status code. Git status codes are ASCII characters ('M', 'A', 'D', 'R', 'C').
        let first_byte = status_bytes[0];
        let x = if first_byte.is_ascii() {
            first_byte as char
        } else {
            // Defensive fallback for unexpected non-ASCII status
            bytes_to_cow_str(status_bytes)
                .chars()
                .next()
                .ok_or_else(|| "empty status in diff output".to_string())?
        };

        // Source path token
        let from_bytes = iter
            .next()
            .filter(|b| !b.is_empty())
            .ok_or_else(|| format!("missing source path for status `{x}`"))?;
        let from = bytes_to_cow_str(from_bytes);

        // Destination path token for renames ('R') or copies ('C')
        let to = if x == 'R' || x == 'C' {
            let to_bytes = iter
                .next()
                .filter(|b| !b.is_empty())
                .ok_or_else(|| format!("missing destination path for status `{x}`"))?;
            bytes_to_cow_str(to_bytes)
        } else {
            Cow::Borrowed("")
        };

        changes.push(FileChange {
            x,
            y: ' ',
            from,
            to,
        });
    }

    Ok(changes)
}

/// Parse newline-separated `git diff-index --name-status` text with zero copies.
///
/// This exists for hooks and scripts which already have the text and do not
/// want the tool to run Git again. Blank lines are ignored.
pub fn parse_name_status_text<'a>(text: &'a str) -> Result<Vec<FileChange<'a>>, String> {
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

        changes.push(FileChange::borrowed(x, from, to));
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
                FileChange::borrowed('M', "src/main.rs", ""),
                FileChange::borrowed('A', "src/new.rs", ""),
                FileChange::borrowed('R', "src/old.rs", "src/new.rs"),
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

    #[test]
    fn zero_copy_slice_pointer_identity() {
        let input = b"M\0src/main.rs\0R100\0src/old.rs\0src/new.rs\0";
        let changes = parse_name_status_z(input).unwrap();

        // Verify that the parsed Cow strings are borrowed directly from `input`
        match &changes[0].from {
            Cow::Borrowed(s) => {
                // Ensure slice pointer points into `input` buffer
                let s_ptr = s.as_ptr();
                let input_start = input.as_ptr();
                let input_end = unsafe { input_start.add(input.len()) };
                assert!(s_ptr >= input_start && s_ptr < input_end);
            }
            Cow::Owned(_) => panic!("expected Cow::Borrowed for ASCII path"),
        }

        match &changes[1].to {
            Cow::Borrowed(s) => {
                let s_ptr = s.as_ptr();
                let input_start = input.as_ptr();
                let input_end = unsafe { input_start.add(input.len()) };
                assert!(s_ptr >= input_start && s_ptr < input_end);
            }
            Cow::Owned(_) => panic!("expected Cow::Borrowed for destination path"),
        }
    }

    #[test]
    fn zero_copy_changes_from_buffer() {
        let input = b"M\0src/main.rs\0A\0src/new.rs\0".to_vec();
        let buffer: Arc<[u8]> = Arc::from(input.into_boxed_slice());
        let changes = Changes::from_buffer(buffer.clone(), true).unwrap();

        assert!(changes.staged);
        assert_eq!(changes.files.len(), 2);
        assert_eq!(changes.files[0].from, "src/main.rs");
        assert_eq!(changes.files[1].from, "src/new.rs");

        // Clone should share the backing Arc buffer without re-allocating strings
        let cloned = changes.clone();
        assert_eq!(cloned.files.len(), 2);
        assert_eq!(cloned.files[0].from, "src/main.rs");
    }

    #[test]
    fn handles_invalid_utf8_fallback() {
        // Invalid UTF-8 sequence [0xFF, 0xFE] in filename
        let input = b"M\0docs/\xFF\xFEfile.md\0";
        let changes = parse_name_status_z(input).unwrap();

        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].from, Cow::Owned(_)));
        assert!(changes[0].from.starts_with("docs/"));
    }
}
