//! Build the commit message from parsed file changes.
//!
//! This is the Rust equivalent of `prepareCommitMsg.ts`.

use crate::constants::AGGREGATE_MIN;
use crate::conventional::{Action, ConventionalType, get_convention_type, lookup_action};
use crate::git::FileChange;
use crate::message::{count_files_desc, named_files_desc, one_change};
use crate::policy::Policy;

/// Type used when the Conventional Commit type cannot be inferred.
const DEFAULT_TYPE: &str = "chore";

/// How a generated message should be formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageFormat {
    /// `type: description`, as required by Conventional Commits.
    Conventional,
    /// The description only, without a generated type.
    Plain,
}

/// A Conventional Commit prefix plus a description.
///
/// The prefix is a string rather than an enum so that custom types from a
/// commitlint configuration (for example `wip` or `deps`) can be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvCommitMsg {
    pub type_prefix: String,
    pub description: String,
}

/// Pieces recovered from an existing commit message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgPieces {
    pub custom_prefix: String,
    pub type_prefix: String,
    pub description: String,
}

/// Join two strings with a space, ignoring empty or identical values.
pub fn join_with_space(first: &str, second: &str) -> String {
    let first = first.trim();
    let second = second.trim();

    if first == second {
        return first.to_string();
    }

    format!("{first} {second}").trim().to_string()
}

/// Join two strings with a colon and space.
pub fn join_with_colon(first: &str, second: &str) -> String {
    format!("{first}: {second}")
}

/// Pick the Conventional Commit type for one change.
pub fn prefix_from_change(change: &FileChange) -> Result<ConventionalType, String> {
    let action = lookup_action(change.x)?;

    Ok(get_convention_type(action, &change.from))
}

/// Generate the prefix and description for a single change.
pub fn msg_one(change: &FileChange) -> Result<ConvCommitMsg, String> {
    Ok(ConvCommitMsg {
        type_prefix: prefix_from_change(change)?.as_str().to_string(),
        description: one_change(change)?,
    })
}

/// Reduce several types to one.
///
/// - No items gives `Unknown`.
/// - Identical items keep that type.
/// - Any `build(deps)` wins over differing types, so a lockfile change is not
///   lost when `package.json` also changed.
/// - Everything else is `Unknown`.
pub fn collapse(types: &[ConventionalType]) -> ConventionalType {
    match types {
        [] => ConventionalType::Unknown,
        [first, rest @ ..] if rest.iter().all(|other| other == first) => *first,
        _ if types.contains(&ConventionalType::BuildDependencies) => {
            ConventionalType::BuildDependencies
        }
        _ => ConventionalType::Unknown,
    }
}

/// Whether a file path is a markdown file (.md or .markdown).
pub fn is_markdown_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// Whether a file change is for a markdown file.
pub fn is_markdown_change(change: &FileChange) -> bool {
    is_markdown_file(&change.from) && (change.to.is_empty() || is_markdown_file(&change.to))
}

/// Whether all file changes in the list are markdown files.
pub fn all_markdown_files(changes: &[FileChange]) -> bool {
    !changes.is_empty() && changes.iter().all(is_markdown_change)
}

/// Generate the prefix and description for a few named changes.
pub fn msg_named(changes: &[FileChange]) -> Result<ConvCommitMsg, String> {
    let types: Vec<ConventionalType> = changes
        .iter()
        .map(prefix_from_change)
        .collect::<Result<_, _>>()?;

    let type_prefix = if all_markdown_files(changes) {
        "docs".to_string()
    } else {
        collapse(&types).as_str().to_string()
    };

    Ok(ConvCommitMsg {
        type_prefix,
        description: named_files_desc(changes),
    })
}

/// Generate the description for many changes using counts.
///
/// The type is inferred by collapsing the types of all changes. It is left
/// empty when they differ or cannot be determined; the formatter supplies a
/// fallback when Conventional Commits is enabled.
pub fn msg_count(changes: &[FileChange]) -> ConvCommitMsg {
    let types: Vec<ConventionalType> = changes
        .iter()
        .map(|change| {
            lookup_action(change.x)
                .map(|action| get_convention_type(action, &change.from))
                .unwrap_or(ConventionalType::Unknown)
        })
        .collect();

    let type_prefix = if all_markdown_files(changes) {
        "docs".to_string()
    } else {
        collapse(&types).as_str().to_string()
    };

    ConvCommitMsg {
        type_prefix,
        description: count_files_desc(changes),
    }
}

/// Describe one or more changes.
///
/// One file is described in full, up to [`AGGREGATE_MIN`] files are named, and
/// larger sets are summarised by count.
pub fn msg_from_changes(changes: &[FileChange]) -> Result<ConvCommitMsg, String> {
    match changes {
        [] => Ok(msg_count(changes)),
        [only] => msg_one(only),
        _ if changes.len() < AGGREGATE_MIN => msg_named(changes),
        _ => Ok(msg_count(changes)),
    }
}

/// Format a message as `type: description`, or just the description when the
/// type is unknown.
pub fn format_msg(message: &ConvCommitMsg) -> String {
    if message.type_prefix.is_empty() {
        return message.description.clone();
    }

    join_with_colon(&message.type_prefix, &message.description)
}

/// Split an existing commit message into its prefixes and description.
pub fn split_msg(message: &str) -> MsgPieces {
    let (prefixes, description) = match message.split_once(':') {
        Some((prefixes, description)) => (prefixes, description.trim()),
        None => ("", message.trim()),
    };

    let (custom_prefix, type_prefix) = match prefixes.rsplit_once(' ') {
        Some((custom, kind)) => (custom, kind),
        None => ("", prefixes),
    };

    MsgPieces {
        custom_prefix: custom_prefix.to_string(),
        type_prefix: type_prefix.to_string(),
        description: description.to_string(),
    }
}

/// Combine generated pieces with pieces from an existing message.
pub fn join_old_and_new(auto: &ConvCommitMsg, old: &MsgPieces) -> String {
    let type_prefix = if !old.type_prefix.is_empty() {
        old.type_prefix.clone()
    } else {
        auto.type_prefix.clone()
    };

    let description = join_with_space(&auto.description, &old.description);

    if type_prefix.is_empty() {
        return description;
    }

    let prefix = join_with_space(&old.custom_prefix, &type_prefix);

    join_with_colon(&prefix, &description)
}

/// Combine an existing message with the auto-generated values.
pub fn combine_old_and_new(auto_type: &str, auto_desc: &str, old_msg: &str) -> String {
    let auto = ConvCommitMsg {
        type_prefix: auto_type.to_string(),
        description: auto_desc.to_string(),
    };

    if old_msg.is_empty() {
        return format_msg(&auto);
    }

    join_old_and_new(&auto, &split_msg(old_msg))
}

/// Merge generated pieces with an existing message.
fn merge_message(auto: &ConvCommitMsg, old_msg: Option<&str>) -> String {
    match old_msg {
        Some(old) if !old.is_empty() => {
            combine_old_and_new(&auto.type_prefix, &auto.description, old)
        }
        _ => format_msg(auto),
    }
}

/// Apply the requested message format before commitlint rules are applied.
fn apply_format(message: &mut ConvCommitMsg, format: MessageFormat) {
    if message.description.is_empty() {
        return;
    }

    match format {
        MessageFormat::Conventional => {
            if message.type_prefix.is_empty() {
                message.type_prefix = DEFAULT_TYPE.to_string();
            }
        }
        MessageFormat::Plain => message.type_prefix.clear(),
    }
}

/// Generate a Conventional Commit message, optionally reusing an existing one.
///
/// An existing type prefix wins over the generated one, and any existing
/// description is kept after the generated description.
pub fn generate_msg(changes: &[FileChange], old_msg: Option<&str>) -> Result<String, String> {
    generate_msg_with_policy(
        changes,
        old_msg,
        &Policy::default(),
        MessageFormat::Conventional,
    )
}

/// Generate a message which follows a [`Policy`].
///
/// The format controls whether a Conventional Commit type is included. The
/// policy can restrict types and scopes, change their case, and limit the
/// header length. When a long named-file message does not fit, the generator
/// falls back to a shorter counted description before truncating anything.
pub fn generate_msg_with_policy(
    changes: &[FileChange],
    old_msg: Option<&str>,
    policy: &Policy,
    format: MessageFormat,
) -> Result<String, String> {
    let mut auto = msg_from_changes(changes)?;
    apply_format(&mut auto, format);
    policy.apply_to_type(&mut auto);

    let message = merge_message(&auto, old_msg);

    if policy.fits(&message) || changes.len() < 2 {
        return Ok(policy.fit(&message));
    }

    let mut counted = msg_count(changes);
    apply_format(&mut counted, format);
    policy.apply_to_type(&mut counted);

    let counted_message = merge_message(&counted, old_msg);

    Ok(policy.fit(&counted_message))
}

/// Describe the action of a change as a word, used by tests and hooks.
pub fn action_of(change: &FileChange) -> Result<Action, String> {
    lookup_action(change.x)
}

const STANDARD_CONVENTIONAL_TYPES: [&str; 11] = [
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// Format the subject start to follow conventional commit casing:
/// If the first word starts with an uppercase letter followed by lowercase
/// (e.g. `Add`, `Fix`, `Implement`), lowercase the initial character.
/// Acronyms (e.g. `CLI`, `API`, `JSON`) are preserved.
pub fn format_subject_start(subject: &str) -> String {
    let mut chars = subject.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return String::new(),
    };
    let second = chars.next();
    if first.is_ascii_uppercase() && second.is_some_and(|c| c.is_ascii_lowercase()) {
        let mut res = first.to_ascii_lowercase().to_string();
        if let Some(s) = second {
            res.push(s);
        }
        res.push_str(chars.as_str());
        res
    } else {
        subject.to_string()
    }
}

/// Check if a header string already has a valid Conventional Commit prefix
/// (such as `feat: ...`, `fix(cli): ...`, or `chore!: ...`).
pub fn has_conventional_prefix(header: &str, policy: &Policy) -> bool {
    let Some((prefix, _)) = header.split_once(": ") else {
        return false;
    };

    let prefix = prefix.trim();
    let (type_part, scope) = crate::policy::split_type_scope(prefix);

    if type_part.is_empty() {
        return false;
    }

    if !type_part
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return false;
    }

    if let Some(s) = scope
        && (s.is_empty()
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.'))
    {
        return false;
    }

    let type_lower = type_part.to_ascii_lowercase();

    if let Some(allowed) = &policy.types {
        allowed.iter().any(|t| t.eq_ignore_ascii_case(&type_lower))
    } else {
        STANDARD_CONVENTIONAL_TYPES.contains(&type_lower.as_str())
    }
}

/// Infer the Conventional Commit type/scope prefix from file changes and policy.
pub fn infer_type_prefix(changes: &[FileChange], policy: &Policy) -> String {
    let mut auto = msg_from_changes(changes).unwrap_or_else(|_| msg_count(changes));

    if all_markdown_files(changes) {
        auto.type_prefix = "docs".to_string();
    } else if auto.type_prefix.is_empty() {
        let types: Vec<ConventionalType> = changes
            .iter()
            .filter_map(|c| {
                lookup_action(c.x)
                    .map(|action| get_convention_type(action, &c.from))
                    .ok()
            })
            .collect();

        let primary = if types.contains(&ConventionalType::Feat) {
            ConventionalType::Feat
        } else if types.contains(&ConventionalType::Fix) {
            ConventionalType::Fix
        } else if types.contains(&ConventionalType::Refactor) {
            ConventionalType::Refactor
        } else if types.contains(&ConventionalType::Docs) {
            ConventionalType::Docs
        } else if types.contains(&ConventionalType::Test) {
            ConventionalType::Test
        } else if types.contains(&ConventionalType::BuildDependencies) {
            ConventionalType::BuildDependencies
        } else {
            ConventionalType::Chore
        };

        auto.type_prefix = primary.as_str().to_string();
    }

    policy.apply_to_type(&mut auto);

    if auto.type_prefix.is_empty() {
        policy
            .fallback_type()
            .unwrap_or_else(|| "chore".to_string())
    } else {
        auto.type_prefix
    }
}

/// Check if the text includes keywords indicating a new feature:
/// "add", "create", "implement" (and their grammatical variants).
pub fn includes_feat_keyword(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for token in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        if token == "add"
            || (token.starts_with("add")
                && !token.starts_with("address")
                && !token.starts_with("addict")
                && !token.starts_with("adder"))
        {
            return true;
        }
        if token.starts_with("creat") {
            return true;
        }
        if token.starts_with("implement") {
            return true;
        }
    }
    false
}

/// Infer the Conventional Commit type/scope prefix for a chat message.
///
/// When all changed files are markdown files, the prefix is "docs".
/// Otherwise, when the chat subject/message includes "add", "create", or "implement",
/// the prefix is "feat". Otherwise, it falls back to inferring the type from
/// the changed files.
pub fn infer_chat_type_prefix(subject: &str, changes: &[FileChange], policy: &Policy) -> String {
    if all_markdown_files(changes) {
        let mut auto = ConvCommitMsg {
            type_prefix: "docs".to_string(),
            description: String::new(),
        };
        policy.apply_to_type(&mut auto);
        if !auto.type_prefix.is_empty() {
            return auto.type_prefix;
        }
    }

    if includes_feat_keyword(subject) {
        let mut auto = ConvCommitMsg {
            type_prefix: "feat".to_string(),
            description: String::new(),
        };
        policy.apply_to_type(&mut auto);
        if !auto.type_prefix.is_empty() {
            return auto.type_prefix;
        }
    }

    infer_type_prefix(changes, policy)
}

/// Format a chat subject to follow Conventional Commit policy when required.
pub fn format_chat_header(subject: &str, changes: &[FileChange], policy: &Policy) -> String {
    if policy.is_empty() {
        return subject.to_string();
    }

    let header = if has_conventional_prefix(subject, policy) {
        subject.to_string()
    } else {
        let prefix = infer_chat_type_prefix(subject, changes, policy);
        let formatted = format_subject_start(subject);
        format!("{prefix}: {formatted}")
    };

    policy.fit(&header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileChange;

    fn change(x: char, from: &str, to: &str) -> FileChange {
        FileChange::new(x, from, to)
    }

    fn single(x: char, path: &str) -> String {
        generate_msg(&[change(x, path, "")], None).unwrap()
    }

    fn plain(changes: &[FileChange], old_msg: Option<&str>) -> String {
        generate_msg_with_policy(changes, old_msg, &Policy::default(), MessageFormat::Plain)
            .unwrap()
    }

    #[test]
    fn joins_strings() {
        assert_eq!(join_with_space("abc", "def"), "abc def");
        assert_eq!(join_with_space(" abc", "def "), "abc def");
        assert_eq!(join_with_space("abc", ""), "abc");
        assert_eq!(join_with_space("", "abc def"), "abc def");
        assert_eq!(join_with_space("abc def", " abc def "), "abc def");
        assert_eq!(join_with_colon("feat", "add thing"), "feat: add thing");
    }

    #[test]
    fn collapses_types() {
        assert_eq!(collapse(&[]), ConventionalType::Unknown);
        assert_eq!(collapse(&[ConventionalType::Feat]), ConventionalType::Feat);
        assert_eq!(
            collapse(&[ConventionalType::Feat, ConventionalType::Feat]),
            ConventionalType::Feat
        );
        assert_eq!(
            collapse(&[ConventionalType::Feat, ConventionalType::Docs]),
            ConventionalType::Unknown
        );
        assert_eq!(
            collapse(&[ConventionalType::Feat, ConventionalType::BuildDependencies,]),
            ConventionalType::BuildDependencies
        );
        assert_eq!(
            collapse(&[ConventionalType::Docs, ConventionalType::BuildDependencies]),
            ConventionalType::BuildDependencies
        );
    }

    #[test]
    fn formats_one_change() {
        assert_eq!(single('A', "foo.txt"), "feat: create foo.txt");
        assert_eq!(single('A', "docs/dev.md"), "docs: create docs/dev.md");
        assert_eq!(single('M', "src/main.rs"), "chore: update src/main.rs");
        assert_eq!(single('M', "foo.txt"), "chore: update foo.txt");
        assert_eq!(single('M', "README.md"), "docs: update README.md");
        assert_eq!(single('D', "foo.txt"), "chore: delete foo.txt");
        assert_eq!(
            single('A', "package-lock.json"),
            "build(deps): create package-lock.json"
        );
    }

    #[test]
    fn formats_two_changes() {
        let changes = [
            change('M', "src/main.rs", ""),
            change('M', "src/lib.rs", ""),
        ];

        assert_eq!(
            generate_msg(&changes, None).unwrap(),
            "chore: update src/main.rs and src/lib.rs"
        );
    }

    #[test]
    fn falls_back_to_counts_for_mixed_actions() {
        let changes = [
            change('M', "src/main.rs", ""),
            change('A', "src/new.rs", ""),
        ];

        assert_eq!(
            generate_msg(&changes, None).unwrap(),
            "chore: update 1 file and create 1 file"
        );
    }

    #[test]
    fn counts_five_or_more_files() {
        let changes: Vec<FileChange> = (0..5)
            .map(|index| change('M', &format!("src/file{index}.rs"), ""))
            .collect();

        assert_eq!(
            generate_msg(&changes, None).unwrap(),
            "chore: update 5 files"
        );
    }

    #[test]
    fn infers_a_type_for_counted_messages() {
        let created: Vec<FileChange> = (0..5)
            .map(|index| change('A', &format!("src/file{index}.rs"), ""))
            .collect();
        let docs: Vec<FileChange> = (0..5)
            .map(|index| change('M', &format!("docs/doc{index}.md"), ""))
            .collect();

        assert_eq!(
            generate_msg(&created, None).unwrap(),
            "feat: create 5 files"
        );
        assert_eq!(generate_msg(&docs, None).unwrap(), "docs: update 5 files");
    }

    #[test]
    fn formats_plain_messages_without_a_type() {
        assert_eq!(plain(&[change('A', "foo.txt", "")], None), "create foo.txt");
        assert_eq!(plain(&[change('M', "foo.txt", "")], None), "update foo.txt");
        assert_eq!(
            plain(&[change('M', "README.md", "")], None),
            "update README.md"
        );
    }

    #[test]
    fn keeps_an_explicit_prefix_in_plain_mode() {
        assert_eq!(
            plain(&[change('M', "foo.txt", "")], Some("fix: rebuild cache")),
            "fix: update foo.txt rebuild cache"
        );
    }

    #[test]
    fn handles_renames() {
        assert_eq!(
            generate_msg(&[change('R', "src/a.rs", "src/b.rs")], None).unwrap(),
            "chore: rename a.rs to b.rs"
        );
    }

    #[test]
    fn splits_existing_messages() {
        assert_eq!(
            split_msg("feat: add thing"),
            MsgPieces {
                custom_prefix: String::new(),
                type_prefix: "feat".to_string(),
                description: "add thing".to_string(),
            }
        );
        assert_eq!(
            split_msg("JIRA-1 feat: add thing"),
            MsgPieces {
                custom_prefix: "JIRA-1".to_string(),
                type_prefix: "feat".to_string(),
                description: "add thing".to_string(),
            }
        );
        assert_eq!(
            split_msg("just a description"),
            MsgPieces {
                custom_prefix: String::new(),
                type_prefix: String::new(),
                description: "just a description".to_string(),
            }
        );
        assert_eq!(
            split_msg("feat: add: thing"),
            MsgPieces {
                custom_prefix: String::new(),
                type_prefix: "feat".to_string(),
                description: "add: thing".to_string(),
            }
        );
    }

    #[test]
    fn keeps_existing_prefix_and_description() {
        assert_eq!(
            generate_msg(&[change('A', "foo.txt", "")], Some("feat: my words")).unwrap(),
            "feat: create foo.txt my words"
        );
        assert_eq!(
            generate_msg(&[change('M', "foo.txt", "")], Some("fix: rebuild cache")).unwrap(),
            "fix: update foo.txt rebuild cache"
        );
        assert_eq!(
            generate_msg(&[change('M', "foo.txt", "")], Some("JIRA-1 chore: tidy")).unwrap(),
            "JIRA-1 chore: update foo.txt tidy"
        );
        assert_eq!(
            generate_msg(&[change('M', "foo.txt", "")], Some("manual description")).unwrap(),
            "chore: update foo.txt manual description"
        );
    }

    #[test]
    fn treats_custom_prefix_without_type_as_the_prefix() {
        assert_eq!(
            generate_msg(&[change('A', "foo.txt", "")], Some("JIRA-1: words")).unwrap(),
            "JIRA-1: create foo.txt words"
        );
    }

    #[test]
    fn ignores_empty_old_message() {
        assert_eq!(
            generate_msg(&[change('A', "foo.txt", "")], Some("")).unwrap(),
            "feat: create foo.txt"
        );
    }

    #[test]
    fn exposes_actions() {
        assert_eq!(action_of(&change('M', "a", "")).unwrap(), Action::Update);
        assert!(action_of(&change('?', "a", "")).is_err());
    }

    #[test]
    fn handles_zero_changes() {
        assert_eq!(generate_msg(&[], None).unwrap(), "");
    }

    #[test]
    fn tests_has_conventional_prefix() {
        let policy = Policy::default();
        assert!(has_conventional_prefix("feat: add thing", &policy));
        assert!(has_conventional_prefix("fix(cli): add thing", &policy));
        assert!(has_conventional_prefix("chore!: breaking", &policy));
        assert!(has_conventional_prefix("build(deps)!: bump", &policy));

        assert!(!has_conventional_prefix("Add chat import", &policy));
        assert!(!has_conventional_prefix(
            "if current path is \"./\" then replace message with:...",
            &policy
        ));
        assert!(!has_conventional_prefix("Note: please check", &policy));
        assert!(!has_conventional_prefix("docs:update", &policy));
    }

    #[test]
    fn formats_subject_start_casing() {
        assert_eq!(format_subject_start("Add chat import"), "add chat import");
        assert_eq!(format_subject_start("Fix null check"), "fix null check");
        assert_eq!(format_subject_start("CLI option"), "CLI option");
        assert_eq!(format_subject_start("already lower"), "already lower");
    }

    #[test]
    fn formats_chat_header_with_policy() {
        let policy = Policy {
            types: Some(vec!["feat".to_string(), "docs".to_string()]),
            strip_full_stop: Some(".".to_string()),
            header_max_length: Some(120),
            ..Policy::default()
        };

        let changes = [change('M', "src/chat.rs", "")];

        // Missing prefix gets auto-added and trailing dots/colons stripped
        assert_eq!(
            format_chat_header(
                "if current path is \"./\" then replace message with:...",
                &changes,
                &policy
            ),
            "feat: if current path is \"./\" then replace message with"
        );

        // Capitalized subject gets lowercased
        assert_eq!(
            format_chat_header("Add chat import", &changes, &policy),
            "feat: add chat import"
        );

        // Existing prefix is preserved
        assert_eq!(
            format_chat_header("docs: update usage guide.", &changes, &policy),
            "docs: update usage guide"
        );

        // Empty policy leaves subject as-is
        assert_eq!(
            format_chat_header("raw subject", &changes, &Policy::default()),
            "raw subject"
        );

        // When all changed files are markdown, it uses docs prefix even with feat keywords
        let doc_changes = [change('M', "README.md", "")];
        assert_eq!(
            format_chat_header(
                "add option at \"mm.json\" file that auto commit with only `mm` CLI - no",
                &doc_changes,
                &policy
            ),
            "docs: add option at \"mm.json\" file that auto commit with only `mm` CLI - no"
        );
        assert_eq!(
            format_chat_header("create unit test from xlsx", &doc_changes, &policy),
            "docs: create unit test from xlsx"
        );
        assert_eq!(
            format_chat_header("implement auto-add option", &doc_changes, &policy),
            "docs: implement auto-add option"
        );
        assert_eq!(
            format_chat_header("refresh guide contents", &doc_changes, &policy),
            "docs: refresh guide contents"
        );

        // When changes include code files, feat keywords produce feat prefix
        let code_changes = [change('M', "src/cli.rs", "")];
        assert_eq!(
            format_chat_header(
                "add option at \"mm.json\" file that auto commit with only `mm` CLI - no",
                &code_changes,
                &policy
            ),
            "feat: add option at \"mm.json\" file that auto commit with only `mm` CLI - no"
        );
        assert_eq!(
            format_chat_header("create unit test from xlsx", &code_changes, &policy),
            "feat: create unit test from xlsx"
        );
        assert_eq!(
            format_chat_header("implement auto-add option", &code_changes, &policy),
            "feat: implement auto-add option"
        );
    }

    #[test]
    fn detects_feat_keywords() {
        assert!(includes_feat_keyword("add option at mm.json"));
        assert!(includes_feat_keyword("Add option at mm.json"));
        assert!(includes_feat_keyword("create unit test"));
        assert!(includes_feat_keyword("creating new components"));
        assert!(includes_feat_keyword("implement auto-add"));
        assert!(includes_feat_keyword("implementation of cache"));
        assert!(includes_feat_keyword("please add support for codex"));
        assert!(!includes_feat_keyword("address issue in parser"));
        assert!(!includes_feat_keyword("fix bug in login"));
        assert!(!includes_feat_keyword("update README.md"));
    }
}
