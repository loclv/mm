//! Rules which shape a generated message.
//!
//! A [`Policy`] is built from a commitlint configuration and applied to the
//! generated message. Only rules which affect generation are represented here;
//! linting rules about bodies, footers or commit trailers are left untouched
//! because this tool only writes a header.

use crate::generate::ConvCommitMsg;

/// Case rules supported by commitlint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    Lower,
    Upper,
    Camel,
    Kebab,
    Pascal,
    Sentence,
    Snake,
    Start,
}

impl Case {
    /// Parse a commitlint case name.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lower-case" => Some(Self::Lower),
            "upper-case" => Some(Self::Upper),
            "camel-case" => Some(Self::Camel),
            "kebab-case" => Some(Self::Kebab),
            "pascal-case" => Some(Self::Pascal),
            "sentence-case" => Some(Self::Sentence),
            "snake-case" => Some(Self::Snake),
            "start-case" => Some(Self::Start),
            _ => None,
        }
    }
}

/// Generation constraints derived from commitlint rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Allowed types (`type-enum`, `always`).
    pub types: Option<Vec<String>>,
    /// Forbidden types (`type-enum`, `never`).
    pub deny_types: Option<Vec<String>>,
    pub type_case: Option<Case>,
    /// `type-empty` is `never`: a type is required.
    pub type_required: bool,
    /// `type-empty` is `always`: no type may be used.
    pub type_forbidden: bool,
    pub type_max_length: Option<usize>,

    /// Allowed scopes (`scope-enum`, `always`).
    pub scopes: Option<Vec<String>>,
    /// Forbidden scopes (`scope-enum`, `never`).
    pub deny_scopes: Option<Vec<String>>,
    pub scope_case: Option<Case>,
    /// `scope-empty` is `always`: scopes may not be used.
    pub scope_forbidden: bool,
    /// `scope-empty` is `never`: a scope is required.
    pub scope_required: bool,

    pub header_max_length: Option<usize>,
    pub subject_max_length: Option<usize>,
    /// `subject-full-stop` / `header-full-stop` with `never`.
    pub strip_full_stop: Option<String>,
}

/// Types to try when the inferred type is not allowed, most useful first.
const FALLBACK_TYPES: [&str; 11] = [
    "chore", "feat", "fix", "docs", "build", "ci", "test", "refactor", "style", "perf", "revert",
];

impl Policy {
    /// Whether this policy changes anything at all.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Fit the generated type and scope to the policy.
    pub fn apply_to_type(&self, message: &mut ConvCommitMsg) {
        let (type_part, scope) = split_type_scope(&message.type_prefix);

        let type_part = self.choose_type(&type_part);
        let scope = if type_part.is_empty() {
            None
        } else {
            self.choose_scope(scope)
        };

        message.type_prefix = compose_type_scope(&type_part, scope.as_deref());
    }

    /// Pick an allowed type, or a fallback when the inferred one is not allowed.
    fn choose_type(&self, candidate: &str) -> String {
        if self.type_forbidden {
            return String::new();
        }

        let mut chosen = candidate.to_string();

        if chosen.is_empty() {
            // A type-enum rule does not require a type by itself. Only add one
            // when `type-empty` says it is needed.
            if !self.type_required {
                return String::new();
            }

            chosen = self.fallback_type().unwrap_or_else(|| "chore".to_string());
        }

        if !self.type_allowed(&chosen) {
            let fallback = self.fallback_type();

            chosen = match fallback {
                Some(fallback) => fallback,
                // Without a usable fallback, keep the meaningful type rather
                // than producing an invalid empty prefix.
                None if self.type_required => chosen,
                None => String::new(),
            };
        }

        if let Some(max) = self.type_max_length
            && chars(&chosen).count() > max
        {
            chosen = self
                .fallback_type_for_length(max)
                .unwrap_or_else(|| truncate(&chosen, max));
        }

        apply_case(&chosen, self.type_case)
    }

    /// Pick an allowed scope, dropping an unusable one.
    fn choose_scope(&self, scope: Option<String>) -> Option<String> {
        if self.scope_forbidden {
            return None;
        }

        let scope = match scope {
            Some(scope) => Some(scope),
            None if self.scope_required => {
                self.scopes.as_ref().and_then(|list| list.first().cloned())
            }
            None => None,
        }?;

        if !self.scope_allowed(&scope) {
            if self.scope_required {
                return self.scopes.as_ref().and_then(|list| list.first().cloned());
            }

            return None;
        }

        Some(apply_case(&scope, self.scope_case))
    }

    fn type_allowed(&self, value: &str) -> bool {
        self.types
            .as_ref()
            .is_none_or(|types| types.iter().any(|item| item == value))
            && !self
                .deny_types
                .as_ref()
                .is_some_and(|types| types.iter().any(|item| item == value))
    }

    fn scope_allowed(&self, value: &str) -> bool {
        self.scopes
            .as_ref()
            .is_none_or(|scopes| scopes.iter().any(|item| item == value))
            && !self
                .deny_scopes
                .as_ref()
                .is_some_and(|scopes| scopes.iter().any(|item| item == value))
    }

    fn fallback_type(&self) -> Option<String> {
        if let Some(types) = &self.types {
            if let Some(preferred) = FALLBACK_TYPES
                .iter()
                .find(|item| types.iter().any(|value| value == *item) && self.type_allowed(item))
            {
                return Some(preferred.to_string());
            }

            return types.iter().find(|item| self.type_allowed(item)).cloned();
        }

        FALLBACK_TYPES
            .iter()
            .find(|item| self.type_allowed(item))
            .map(|item| item.to_string())
    }

    fn fallback_type_for_length(&self, max: usize) -> Option<String> {
        if let Some(types) = &self.types {
            return types
                .iter()
                .find(|item| self.type_allowed(item) && chars(item).count() <= max)
                .cloned();
        }

        FALLBACK_TYPES
            .iter()
            .find(|item| chars(item).count() <= max)
            .map(|item| item.to_string())
    }

    /// Whether the header already satisfies the length rules.
    pub fn fits(&self, message: &str) -> bool {
        let header = first_line(message);
        let (_, subject) = split_header(header);

        self.header_max_length
            .is_none_or(|max| chars(header).count() <= max)
            && self
                .subject_max_length
                .is_none_or(|max| chars(subject).count() <= max)
    }

    /// Remove a forbidden trailing full stop and enforce length limits.
    ///
    /// Length limits are applied last, so a generated message always has a
    /// single-line header within the configured maximum.
    pub fn fit(&self, message: &str) -> String {
        let header = first_line(message).to_string();
        let (prefix, subject) = split_header(&header);

        let mut subject = subject.to_string();

        if let Some(suffix) = &self.strip_full_stop
            && let Some(trimmed) = subject.strip_suffix(suffix)
        {
            subject = trimmed.trim_end().to_string();
        }

        if let Some(max) = self.subject_max_length {
            subject = truncate(&subject, max);
        }

        if let Some(max) = self.header_max_length {
            let prefix_len = if prefix.is_empty() {
                0
            } else {
                chars(prefix).count() + 2 // ": "
            };
            let available = max.saturating_sub(prefix_len);

            if chars(&subject).count() > available {
                subject = truncate(&subject, available);
            }
        }

        if prefix.is_empty() {
            subject
        } else {
            format!("{prefix}: {subject}").trim_end().to_string()
        }
    }
}

/// Split `build(deps)` into `build` and `deps`.
pub fn split_type_scope(prefix: &str) -> (String, Option<String>) {
    let prefix = prefix.trim_end_matches('!');

    if let Some(open) = prefix.find('(')
        && prefix.ends_with(')')
    {
        return (
            prefix[..open].to_string(),
            Some(prefix[open + 1..prefix.len() - 1].to_string()),
        );
    }

    (prefix.to_string(), None)
}

/// Build `type(scope)`.
pub fn compose_type_scope(type_part: &str, scope: Option<&str>) -> String {
    if type_part.is_empty() {
        return String::new();
    }

    match scope {
        Some(scope) if !scope.is_empty() => format!("{type_part}({scope})"),
        _ => type_part.to_string(),
    }
}

/// Split a header into the type/custom prefix and the subject.
fn split_header(header: &str) -> (&str, &str) {
    match header.split_once(':') {
        Some((prefix, subject)) => (prefix.trim(), subject.trim()),
        None => ("", header.trim()),
    }
}

fn first_line(message: &str) -> &str {
    message.lines().next().unwrap_or("")
}

/// Apply a commitlint case to a value.
fn apply_case(value: &str, case: Option<Case>) -> String {
    let Some(case) = case else {
        return value.to_string();
    };

    let words = split_words(value);

    if words.is_empty() {
        return value.to_string();
    }

    match case {
        Case::Lower => value.to_lowercase(),
        Case::Upper => value.to_uppercase(),
        Case::Kebab => words
            .iter()
            .map(|word| word.to_lowercase())
            .collect::<Vec<_>>()
            .join("-"),
        Case::Snake => words
            .iter()
            .map(|word| word.to_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
        Case::Pascal => words.iter().map(|word| capitalize(word)).collect(),
        Case::Camel => {
            let pascal: String = words.iter().map(|word| capitalize(word)).collect();
            let mut characters = pascal.chars();

            match characters.next() {
                Some(first) => first.to_lowercase().collect::<String>() + characters.as_str(),
                None => String::new(),
            }
        }
        Case::Start => words
            .iter()
            .map(|word| capitalize(word))
            .collect::<Vec<_>>()
            .join(" "),
        Case::Sentence => {
            let lower: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
            let mut joined = lower.join(" ");

            if let Some(first) = joined.chars().next() {
                let mut characters = joined.chars();
                characters.next();
                joined = first.to_uppercase().collect::<String>() + characters.as_str();
            }

            joined
        }
    }
}

/// Split a value into words, including camelCase and PascalCase boundaries.
fn split_words(value: &str) -> Vec<String> {
    let characters: Vec<char> = value.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();

    for (index, character) in characters.iter().enumerate() {
        if !character.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        if character.is_uppercase() && !current.is_empty() {
            let previous = characters[index - 1];
            let next_is_lower = characters
                .get(index + 1)
                .is_some_and(|next| next.is_lowercase());

            if previous.is_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_uppercase() && next_is_lower)
            {
                words.push(std::mem::take(&mut current));
            }
        }

        current.push(*character);
    }

    if !current.is_empty() {
        words.push(current);
    }

    words
}

fn capitalize(word: &str) -> String {
    let mut characters = word.chars();

    match characters.next() {
        Some(first) => {
            first.to_uppercase().collect::<String>() + &characters.as_str().to_lowercase()
        }
        None => String::new(),
    }
}

fn chars(value: &str) -> std::str::Chars<'_> {
    value.chars()
}

/// Truncate to a character count, adding an ellipsis when there is room.
pub fn truncate(value: &str, max: usize) -> String {
    if chars(value).count() <= max {
        return value.to_string();
    }

    if max == 0 {
        return String::new();
    }

    if max <= 3 {
        return chars(value).take(max).collect();
    }

    let mut result: String = chars(value).take(max - 3).collect();
    result.push_str("...");
    result.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(type_prefix: &str, description: &str) -> ConvCommitMsg {
        ConvCommitMsg {
            type_prefix: type_prefix.to_string(),
            description: description.to_string(),
        }
    }

    #[test]
    fn splits_and_composes_scope() {
        assert_eq!(
            split_type_scope("build(deps)"),
            ("build".to_string(), Some("deps".to_string()))
        );
        assert_eq!(split_type_scope("feat"), ("feat".to_string(), None));
        assert_eq!(compose_type_scope("build", Some("deps")), "build(deps)");
        assert_eq!(compose_type_scope("feat", None), "feat");
    }

    #[test]
    fn applies_cases() {
        assert_eq!(apply_case("build", Some(Case::Upper)), "BUILD");
        assert_eq!(apply_case("BuildDeps", Some(Case::Kebab)), "build-deps");
        assert_eq!(apply_case("build-deps", Some(Case::Camel)), "buildDeps");
        assert_eq!(apply_case("build-deps", Some(Case::Pascal)), "BuildDeps");
        assert_eq!(apply_case("build deps", Some(Case::Snake)), "build_deps");
        assert_eq!(apply_case("build deps", Some(Case::Start)), "Build Deps");
        assert_eq!(apply_case("BUILD deps", Some(Case::Sentence)), "Build deps");
        assert_eq!(apply_case("keep", None), "keep");
    }

    #[test]
    fn restricts_types() {
        let policy = Policy {
            types: Some(vec!["chore".to_string(), "feat".to_string()]),
            ..Policy::default()
        };

        let mut msg = message("docs", "update README.md");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "chore");
    }

    #[test]
    fn keeps_allowed_types_and_removes_scope() {
        let policy = Policy {
            types: Some(vec!["build".to_string()]),
            scopes: Some(vec!["api".to_string()]),
            ..Policy::default()
        };

        let mut msg = message("build(deps)", "update Cargo.lock");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "build");
    }

    #[test]
    fn keeps_allowed_scope() {
        let policy = Policy {
            scopes: Some(vec!["deps".to_string()]),
            ..Policy::default()
        };

        let mut msg = message("build(deps)", "update Cargo.lock");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "build(deps)");
    }

    #[test]
    fn forbids_scope() {
        let policy = Policy {
            scope_forbidden: true,
            ..Policy::default()
        };

        let mut msg = message("build(deps)", "update Cargo.lock");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "build");
    }

    #[test]
    fn does_not_add_a_type_when_it_is_optional() {
        let policy = Policy {
            types: Some(vec!["chore".to_string()]),
            ..Policy::default()
        };

        let mut msg = message("", "update 3 files");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "");
    }

    #[test]
    fn requires_a_type() {
        let policy = Policy {
            types: Some(vec!["chore".to_string(), "fix".to_string()]),
            type_required: true,
            ..Policy::default()
        };

        let mut msg = message("", "update 3 files");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "chore");
    }

    #[test]
    fn required_scope_uses_the_first_allowed_scope() {
        let policy = Policy {
            scopes: Some(vec!["api".to_string()]),
            scope_required: true,
            ..Policy::default()
        };

        let mut msg = message("feat", "create foo.rs");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "feat(api)");
    }

    #[test]
    fn forbidding_a_type_also_drops_the_scope() {
        let policy = Policy {
            type_forbidden: true,
            ..Policy::default()
        };

        let mut msg = message("build(deps)", "update Cargo.lock");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "");
    }

    #[test]
    fn forbids_a_type() {
        let policy = Policy {
            type_forbidden: true,
            ..Policy::default()
        };

        let mut msg = message("feat", "create foo.txt");
        policy.apply_to_type(&mut msg);

        assert_eq!(msg.type_prefix, "");
    }

    #[test]
    fn fits_and_truncates_headers() {
        let policy = Policy {
            header_max_length: Some(20),
            ..Policy::default()
        };

        assert!(!policy.fits("feat: create a very long file name.rs"));
        assert_eq!(
            policy.fit("feat: create a very long file name.rs"),
            "feat: create a ve..."
        );
        assert_eq!(policy.fit("feat: short"), "feat: short");
    }

    #[test]
    fn truncates_subjects_without_a_prefix() {
        let policy = Policy {
            header_max_length: Some(10),
            ..Policy::default()
        };

        assert_eq!(policy.fit("update main.rs and lib.rs"), "update ...");
    }

    #[test]
    fn strips_full_stops() {
        let policy = Policy {
            strip_full_stop: Some(".".to_string()),
            ..Policy::default()
        };

        assert_eq!(
            policy.fit("docs: update README.md."),
            "docs: update README.md"
        );
    }

    #[test]
    fn empty_policy_is_detected() {
        assert!(Policy::default().is_empty());
        assert!(
            !Policy {
                header_max_length: Some(72),
                ..Policy::default()
            }
            .is_empty()
        );
    }
}
