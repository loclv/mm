//! Path helpers.
//!
//! These functions deal with paths as strings because Git always reports paths
//! with forward slashes, even on Windows.

use crate::constants::ROOT;

/// File name stems that are likely to appear more than once in a repository.
const REPEAT_FILENAMES: [&str; 3] = ["readme", "index", "__init__.py"];

/// Metadata about a path, derived from the string alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPath {
    /// True when the path has no directory component.
    pub at_root: bool,
    /// Directory of the path, or [`ROOT`] when the file is at the top level.
    pub dir_path: String,
    /// File name including the extension.
    pub name: String,
    /// Extension including the dot, or an empty string when there is none.
    pub extension: String,
}

impl SplitPath {
    /// Split a path into its directory, name and extension.
    pub fn new(file_path: &str) -> Self {
        let (dir, name) = match file_path.rfind('/') {
            Some(index) => (&file_path[..index], &file_path[index + 1..]),
            None => ("", file_path),
        };

        let at_root = dir.is_empty();

        Self {
            at_root,
            dir_path: if at_root {
                ROOT.to_string()
            } else {
                dir.to_string()
            },
            name: name.to_string(),
            extension: extension_of(name).to_string(),
        }
    }
}

/// Return the extension of a file name, including the dot.
///
/// Matches `path.extname` from Node: a leading dot is not an extension, so
/// `.gitignore` has no extension, while `foo.` has the extension `.`.
fn extension_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(index) if index > 0 => &name[index..],
        _ => "",
    }
}

/// Split a path into its directory, name and extension.
pub fn split_path(file_path: &str) -> SplitPath {
    SplitPath::new(file_path)
}

/// Whether a pair of paths represents a move, a rename, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOrRename {
    Move,
    Rename,
    MoveAndRename,
}

impl MoveOrRename {
    /// Lower-case description used in commit messages.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Rename => "rename",
            Self::MoveAndRename => "move and rename",
        }
    }
}

/// Determine if a pair of paths represents a move, a rename, or both.
pub fn move_or_rename_from_paths(old_path: &SplitPath, new_path: &SplitPath) -> MoveOrRename {
    if old_path.name == new_path.name {
        MoveOrRename::Move
    } else if old_path.dir_path == new_path.dir_path {
        MoveOrRename::Rename
    } else {
        MoveOrRename::MoveAndRename
    }
}

/// Wrap a value in single quotes when it contains a space.
///
/// The repository root label is never quoted.
pub fn quote_for_spaces(value: &str) -> String {
    if value.contains(' ') && value != ROOT {
        format!("'{value}'")
    } else {
        value.to_string()
    }
}

/// Change a file path to a short, readable form.
///
/// Usually only the file name is kept. Names which tend to be repeated in a
/// repository (such as `README.md`) keep their full path so that the message is
/// unambiguous.
pub fn friendly_file(file_path: &str) -> String {
    let split = SplitPath::new(file_path);
    let lower_name = split.name.to_lowercase();

    if REPEAT_FILENAMES
        .iter()
        .any(|prefix| lower_name.starts_with(prefix))
    {
        return quote_for_spaces(file_path);
    }

    quote_for_spaces(&split.name)
}

/// Join items with commas and an "and" before the last item.
///
/// For example `["a", "b", "c"]` becomes `a, b and c`.
pub fn join(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Express a list of paths in plain English.
///
/// # Panics
///
/// Panics when called with no paths, because there is nothing to describe.
pub fn human_list(paths: &[String]) -> String {
    assert!(!paths.is_empty(), "Expected at least one path, got zero");

    if paths.len() == 1 {
        return friendly_file(&paths[0]);
    }

    let friendly: Vec<String> = paths.iter().map(|path| friendly_file(path)).collect();

    join(&friendly)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_paths() {
        assert_eq!(
            SplitPath::new("foo.txt"),
            SplitPath {
                at_root: true,
                dir_path: ROOT.to_string(),
                name: "foo.txt".to_string(),
                extension: ".txt".to_string(),
            }
        );

        assert_eq!(
            SplitPath::new("docs/foo.md"),
            SplitPath {
                at_root: false,
                dir_path: "docs".to_string(),
                name: "foo.md".to_string(),
                extension: ".md".to_string(),
            }
        );
    }

    #[test]
    fn uses_node_style_extensions() {
        assert_eq!(SplitPath::new(".gitignore").extension, "");
        assert_eq!(SplitPath::new("foo.").extension, ".");
        assert_eq!(SplitPath::new("a/b/archive.tar.gz").extension, ".gz");
        assert_eq!(SplitPath::new("Makefile").extension, "");
    }

    #[test]
    fn quotes_values_with_spaces() {
        assert_eq!(quote_for_spaces("foo bar.txt"), "'foo bar.txt'");
        assert_eq!(quote_for_spaces("foo.txt"), "foo.txt");
        assert_eq!(quote_for_spaces(ROOT), ROOT);
    }

    #[test]
    fn shortens_paths_where_possible() {
        assert_eq!(friendly_file("src/main.rs"), "main.rs");
        assert_eq!(friendly_file("docs/README.md"), "docs/README.md");
        assert_eq!(friendly_file("README.md"), "README.md");
        assert_eq!(friendly_file("a/foo bar.txt"), "'foo bar.txt'");
    }

    #[test]
    fn joins_lists() {
        assert_eq!(join(&[]), "");
        assert_eq!(join(&["a".to_string()]), "a");
        assert_eq!(join(&["a".to_string(), "b".to_string()]), "a and b");
        assert_eq!(
            join(&["a".to_string(), "b".to_string(), "c".to_string()]),
            "a, b and c"
        );
    }

    #[test]
    fn builds_human_lists() {
        assert_eq!(human_list(&["src/main.rs".to_string()]), "main.rs");
        assert_eq!(
            human_list(&["src/main.rs".to_string(), "src/lib.rs".to_string()]),
            "main.rs and lib.rs"
        );
    }

    #[test]
    fn detects_moves_and_renames() {
        let old = SplitPath::new("src/foo.rs");
        let new = SplitPath::new("lib/foo.rs");
        assert_eq!(move_or_rename_from_paths(&old, &new), MoveOrRename::Move);

        let new = SplitPath::new("src/bar.rs");
        assert_eq!(move_or_rename_from_paths(&old, &new), MoveOrRename::Rename);

        let new = SplitPath::new("lib/bar.rs");
        assert_eq!(
            move_or_rename_from_paths(&old, &new),
            MoveOrRename::MoveAndRename
        );
    }
}
