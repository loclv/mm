//! Rust rewrite of the Auto Commit Message CLI.
//!
//! The crate is split into small modules that mirror the original TypeScript
//! logic:
//!
//! - [`git`] runs Git and parses `diff-index` output.
//! - [`paths`] turns file paths into human-friendly text.
//! - [`conventional`] picks the Conventional Commit type for a file.
//! - [`message`] describes one file change or a group of changes.
//! - [`generate`] assembles the final commit message.
//! - [`config`] reads the optional `mm.json` project settings.
//! - [`cli`] implements the `mm` command-line interface.

pub mod cli;
pub mod commitlint;
pub mod config;
pub mod conventional;
pub mod generate;
pub mod git;
pub mod message;
pub mod paths;
pub mod policy;

/// Constants shared across modules.
pub mod constants {
    /// Use file names up to this many files, then switch to counts.
    pub const AGGREGATE_MIN: usize = 5;

    /// Human-friendly name for the repository root, used in commit messages.
    pub const ROOT: &str = "repo root";

    /// Error shown when Git reports no changed files.
    pub const NO_LINES_MSG: &str = "Unable to generate message as no changes files can be seen.\nTry saving your files or stage any new (untracked) files.";
}
