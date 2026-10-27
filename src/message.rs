//! Describe file changes in words.
//!
//! A single change gets a sentence such as `update foo.txt`. Several changes are
//! either listed by name (up to [`AGGREGATE_MIN`] files) or summarised with
//! counts (`update 5 files and create 2 files`).

use crate::constants::ROOT;
use crate::conventional::{Action, lookup_action};
use crate::git::FileChange;
use crate::paths::{
    friendly_file, human_list, join, move_or_rename_from_paths, quote_for_spaces, split_path,
};

/// Full message for moving and/or renaming a file.
pub fn move_or_rename_msg(old_path: &str, new_path: &str) -> String {
    let old_p = split_path(old_path);
    let new_p = split_path(new_path);

    let from = quote_for_spaces(&old_p.name);

    match move_or_rename_from_paths(&old_p, &new_p) {
        crate::paths::MoveOrRename::Move => {
            format!("move {from} to {}", quote_for_spaces(&new_p.dir_path))
        }
        crate::paths::MoveOrRename::Rename => {
            format!("rename {from} to {}", quote_for_spaces(&new_p.name))
        }
        crate::paths::MoveOrRename::MoveAndRename => {
            let to = quote_for_spaces(&new_p.name);
            let target = if new_p.dir_path == ROOT {
                format!("{to} at {ROOT}")
            } else {
                quote_for_spaces(new_path)
            };

            format!("move and rename {from} to {target}")
        }
    }
}

/// Describe a single file change, e.g. `update foo.txt`.
pub fn one_change(change: &FileChange) -> Result<String, String> {
    let action = lookup_action(change.x)?;

    if action == Action::Rename {
        return Ok(move_or_rename_msg(&change.from, &change.to));
    }

    Ok(format!(
        "{} {}",
        action.as_str(),
        friendly_file(&change.from)
    ))
}

/// Determine if a rename change is a move, a rename, or both.
fn move_or_rename_from_change(change: &FileChange) -> crate::paths::MoveOrRename {
    let old_p = split_path(&change.from);
    let new_p = split_path(&change.to);

    move_or_rename_from_paths(&old_p, &new_p)
}

/// Group changes by action and count them, preserving the order Git reported.
fn count_by_action(changes: &[FileChange]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();

    for change in changes {
        let action = if change.x == 'R' {
            move_or_rename_from_change(change).as_str().to_string()
        } else {
            match lookup_action(change.x) {
                Ok(action) => action.as_str().to_string(),
                Err(_) => change.x.to_string(),
            }
        };

        match counts.iter_mut().find(|(key, _)| *key == action) {
            Some((_, count)) => *count += 1,
            None => counts.push((action, 1)),
        }
    }

    counts
}

/// `file` or `files`, depending on the count.
fn plural_s(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Format one action and its count, e.g. `update 3 files`.
fn format_one(action: &str, count: usize) -> String {
    format!("{action} {count} file{}", plural_s(count))
}

/// Describe changes using counts for each action.
pub fn count_files_desc(changes: &[FileChange]) -> String {
    let messages: Vec<String> = count_by_action(changes)
        .iter()
        .map(|(action, count)| format_one(action, *count))
        .collect();

    join(&messages)
}

/// Describe several changes by listing their names.
///
/// When the actions differ, this falls back to counts so that the message does
/// not claim every file changed in the same way.
pub fn named_files_desc(changes: &[FileChange]) -> String {
    let all_same_action = changes
        .iter()
        .map(|change| change.x)
        .all(|x| x == changes[0].x);

    if !all_same_action {
        return count_files_desc(changes);
    }

    let action = match lookup_action(changes[0].x) {
        Ok(action) => action,
        Err(_) => return count_files_desc(changes),
    };

    let paths: Vec<String> = changes
        .iter()
        .map(|change| change.from.to_string())
        .collect();

    format!("{} {}", action.as_str(), human_list(&paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileChange;

    fn change(x: char, from: &str, to: &str) -> FileChange<'static> {
        FileChange::new(x, from.to_string(), to.to_string())
    }

    #[test]
    fn describes_one_change() {
        assert_eq!(
            one_change(&change('M', "src/main.rs", "")).unwrap(),
            "update src/main.rs"
        );
        assert_eq!(
            one_change(&change('A', "src/new.rs", "")).unwrap(),
            "create src/new.rs"
        );
        assert_eq!(
            one_change(&change('A', "docs/dev.md", "")).unwrap(),
            "create docs/dev.md"
        );
        assert_eq!(one_change(&change('D', "TODO", "")).unwrap(), "delete TODO");
    }

    #[test]
    fn describes_renames() {
        assert_eq!(
            one_change(&change('R', "src/foo.rs", "lib/foo.rs")).unwrap(),
            "move foo.rs to lib"
        );
        assert_eq!(
            one_change(&change('R', "src/foo.rs", "src/bar.rs")).unwrap(),
            "rename foo.rs to bar.rs"
        );
        assert_eq!(
            one_change(&change('R', "src/foo.rs", "lib/bar.rs")).unwrap(),
            "move and rename foo.rs to lib/bar.rs"
        );
        assert_eq!(
            one_change(&change('R', "foo.md", "bar.md")).unwrap(),
            "rename foo.md to bar.md"
        );
    }

    #[test]
    fn lists_names_for_a_few_changes() {
        let changes = [
            change('M', "src/main.rs", ""),
            change('M', "src/lib.rs", ""),
        ];

        assert_eq!(
            named_files_desc(&changes),
            "update src/main.rs and src/lib.rs"
        );
    }

    #[test]
    fn counts_when_actions_differ() {
        let changes = [
            change('M', "src/main.rs", ""),
            change('A', "src/new.rs", ""),
        ];

        assert_eq!(
            count_files_desc(&changes),
            "update 1 file and create 1 file"
        );
    }

    #[test]
    fn counts_moves_and_renames() {
        let changes = [
            change('R', "a/foo.rs", "b/foo.rs"),
            change('R', "a/bar.rs", "a/baz.rs"),
            change('R', "a/x.rs", "b/y.rs"),
        ];

        assert_eq!(
            count_files_desc(&changes),
            "move 1 file, rename 1 file and move and rename 1 file"
        );
    }

    #[test]
    fn uses_empty_action_for_unknown_status() {
        // A lookup error in `one_change` is surfaced, but counts stay readable.
        assert_eq!(count_files_desc(&[change('?', "foo.rs", "")]), "? 1 file");
    }
}
