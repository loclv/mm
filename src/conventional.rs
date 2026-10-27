//! Conventional Commit type detection.
//!
//! The type is inferred from the path alone. A few actions also influence the
//! result: creating an otherwise unknown file is a `feat`, while deleting or
//! renaming a file is a `chore`.

use crate::paths::SplitPath;

/// The action which happened to a file, based on Git's status letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Update,
    Create,
    Delete,
    Rename,
    Copy,
    Unknown,
}

impl Action {
    /// Verb used in a commit message description.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Rename => "rename",
            Self::Copy => "copy",
            Self::Unknown => "",
        }
    }
}

/// Map a `git diff-index` status letter to an [`Action`].
pub fn lookup_action(x: char) -> Result<Action, String> {
    match x {
        'M' => Ok(Action::Update),
        'A' => Ok(Action::Create),
        'D' => Ok(Action::Delete),
        'R' => Ok(Action::Rename),
        'C' => Ok(Action::Copy),
        _ => Err(format!("Unknown ACTION key: {x}")),
    }
}

/// Conventional Commit types used by this tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConventionalType {
    Build,
    BuildDependencies,
    Ci,
    Chore,
    Docs,
    Feat,
    Fix,
    Perf,
    Refactor,
    Revert,
    Style,
    Test,
    Unknown,
}

impl ConventionalType {
    /// Text used as the commit message prefix.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::BuildDependencies => "build(deps)",
            Self::Ci => "ci",
            Self::Chore => "chore",
            Self::Docs => "docs",
            Self::Feat => "feat",
            Self::Fix => "fix",
            Self::Perf => "perf",
            Self::Refactor => "refactor",
            Self::Revert => "revert",
            Self::Style => "style",
            Self::Test => "test",
            Self::Unknown => "",
        }
    }
}

const PACKAGE_NAMES: &[&str] = &[
    // Python
    "requirements.txt",
    "requirements-dev.txt",
    "dev-requirements.txt",
    "requirements-test.txt",
    "test-requirements.txt",
    "Pipfile",
    "Pipefile.lock",
    "poetry.toml",
    "poetry.lock",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    // Ruby
    "Gemfile",
    "Gemfile.lock",
    // NPM
    "package-lock.json",
    "shrinkwrap.json",
    "yarn.lock",
    // PHP
    "composer.json",
    "composer.lock",
    // Go
    "go.mod",
    "go.sum",
    // Rust
    "Cargo.toml",
    "Cargo.lock",
    // Deno
    "deps.ts",
    "test_deps.ts",
    "dev_deps.ts",
    "import_map.json",
];

const BUILD_NAMES: &[&str] = &[
    // Docker
    ".dockerignore",
    "Dockerfile",
    "docker-compose.yml",
    // Make
    "GNUmakefile",
    "makefile",
    "Makefile",
    // Ruby Rake
    "rakefile",
    "Rakefile",
    "rakefile.rb",
    "Rakefile.rb",
    // Not necessarily package-related, but always build-related.
    "package.json",
    // Java
    "gradlew",
    "grailsw",
    "micronaut-cli.yml",
];

const BUILD_EXTENSIONS: &[&str] = &[".gemspec", ".bat", ".gradle"];

const CONFIG_EXTENSIONS: &[&str] = &[
    ".yml",
    ".yaml",
    ".json",
    ".toml",
    ".ini",
    ".cfg",
    ".config.js",
];

const CONFIG_DIRS: &[&str] = &[".vscode"];

const CONFIG_NAMES: &[&str] = &[
    // Git
    ".gitignore",
    // EditorConfig
    ".editorconfig",
    // Python
    ".pylintrc",
    ".isort.cfg",
    ".flake8",
    "pytest.ini",
    ".coveragerc",
    // Node
    ".npmignore",
    ".npmrc",
    ".babelrc",
    ".eslintrc",
    ".browserslistrc",
    "browserslist",
    "rollup.config.js",
    "webpack.config.js",
    "npm-shrinkwrap.json",
    "tsconfig.json",
    "tslint.json",
    // Dotenv
    ".env",
    ".env.dev",
    ".env.prod",
    ".env.tempate",
];

const CI_DIRS: &[&str] = &[".circleci", ".github/workflows"];

const CI_NAMES: &[&str] = &[
    "netlify.toml",
    "travis.yml",
    "tox.ini",
    ".vscodeignore",
    "codecov.yml",
    ".codecov.yml",
    ".codeclimate.yml",
    // Zeit / Vercel
    "now.json",
    ".nowignore",
    "vercel.json",
    ".vercelignore",
    "Procfile",
];

const LICENSE_NAMES: &[&str] = &["LICENSE", "LICENSE.txt", "License.txt", "LICENSE-source"];

const DOC_NAMES: &[&str] = &[
    "readme",
    "readme.md",
    "readme.txt",
    "installation.md",
    "usage.md",
    "development.md",
    "deploy.md",
    // GitHub docs.
    "security.md",
    "contributing.md",
    "changelog.md",
    "releases.md",
    "funding.md",
    "pull_request_template.md",
    "issue_template.md",
    "code_of_conduct.md",
    "maintainers.txt",
    "codeowners",
    // Images.
    "sample.png",
    "sample-1.png",
    "sample-2.png",
    "screenshot.png",
    "preview.png",
];

/// A file path split into the pieces used by the type rules.
#[derive(Debug)]
pub struct ConventionalCommit {
    pub at_root: bool,
    pub dir_path: String,
    pub name: String,
    pub extension: String,
}

impl ConventionalCommit {
    /// Inspect a path.
    pub fn new(file_path: &str) -> Self {
        let split = SplitPath::new(file_path);

        Self {
            at_root: split.at_root,
            dir_path: split.dir_path,
            name: split.name,
            extension: split.extension,
        }
    }

    /// Whether the file is documentation.
    pub fn is_docs_related(&self) -> bool {
        self.extension.eq_ignore_ascii_case(".md")
            || self.extension.eq_ignore_ascii_case(".markdown")
            || self.extension.eq_ignore_ascii_case(".rst")
            || DOC_NAMES.contains(&self.name.to_lowercase().as_str())
            || self.dir_path.starts_with("docs")
    }

    /// Whether the file is a test, mock or coverage configuration.
    fn is_test_related(&self) -> bool {
        const TEST_DIRS: [&str; 6] = ["test", "tests", "spec", "unit", "unit_tests", "__mocks__"];

        if self
            .dir_path
            .split('/')
            .any(|segment| TEST_DIRS.contains(&segment))
        {
            return true;
        }

        let name = &self.name;

        if name == ".coveragerc" {
            return true;
        }

        has_extension(name)
            && (name.starts_with("test_")
                || name.starts_with("spec_")
                || name.contains(".test.")
                || name.contains(".spec."))
    }

    /// Whether the file configures CI.
    fn is_ci_related(&self) -> bool {
        CI_DIRS.contains(&self.dir_path.as_str()) || CI_NAMES.contains(&self.name.as_str())
    }

    /// Whether the file is tool or project configuration.
    fn is_config_related(&self) -> bool {
        CONFIG_EXTENSIONS.contains(&self.extension.as_str())
            || CONFIG_DIRS.contains(&self.dir_path.as_str())
            || CONFIG_NAMES.contains(&self.name.as_str())
            || self.name.contains(".eslintrc")
            || self.name.contains(".eslintignore")
            || self.name.contains(".eslintcache")
            || self.name.contains(".prettier")
            || self.name.contains("tslint")
            || self.name.contains("webpack")
    }

    /// Whether the file manages dependencies.
    fn is_package_related(&self) -> bool {
        PACKAGE_NAMES.contains(&self.name.as_str())
    }

    /// Whether the file is part of the build.
    fn is_build_related(&self) -> bool {
        BUILD_NAMES.contains(&self.name.as_str())
            || BUILD_EXTENSIONS.contains(&self.extension.as_str())
    }

    /// Whether the file is a license.
    fn is_license_related(&self) -> bool {
        LICENSE_NAMES.contains(&self.name.as_str())
    }

    /// Whether the file is a chore (license or configuration).
    fn is_chore_related(&self) -> bool {
        self.is_license_related() || self.is_config_related()
    }

    /// Pick the type, checking the most specific categories first.
    pub fn get_type(&self) -> ConventionalType {
        if self.is_ci_related() {
            return ConventionalType::Ci;
        }
        if self.is_package_related() {
            return ConventionalType::BuildDependencies;
        }
        if self.is_build_related() {
            return ConventionalType::Build;
        }
        if self.is_chore_related() {
            return ConventionalType::Chore;
        }
        if self.is_docs_related() {
            return ConventionalType::Docs;
        }
        if self.is_test_related() {
            return ConventionalType::Test;
        }

        ConventionalType::Unknown
    }
}

/// Whether a file name has an extension (a dot followed by at least one char).
fn has_extension(name: &str) -> bool {
    match name.rfind('.') {
        Some(index) => index > 0 && index + 1 < name.len(),
        None => false,
    }
}

/// Pick the Conventional Commit type for a file change.
pub fn get_convention_type(action: Action, file_path: &str) -> ConventionalType {
    if action == Action::Rename || action == Action::Delete {
        return ConventionalType::Chore;
    }

    let commit = ConventionalCommit::new(file_path);
    if commit.is_docs_related() {
        return ConventionalType::Docs;
    }

    let commit_type = commit.get_type();

    if action == Action::Create && commit_type == ConventionalType::Unknown {
        return ConventionalType::Feat;
    }

    commit_type
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefix(action: char, path: &str) -> ConventionalType {
        get_convention_type(lookup_action(action).unwrap(), path)
    }

    #[test]
    fn new_generic_file_is_a_feature() {
        assert_eq!(prefix('A', "foo.txt"), ConventionalType::Feat);
        assert_eq!(prefix('A', "foo bar.txt"), ConventionalType::Feat);
        assert_eq!(prefix('A', "src/main.rs"), ConventionalType::Feat);
    }

    #[test]
    fn modified_generic_file_is_unknown() {
        assert_eq!(prefix('M', "foo.txt"), ConventionalType::Unknown);
    }

    #[test]
    fn deleted_or_renamed_is_a_chore() {
        assert_eq!(prefix('D', "foo.txt"), ConventionalType::Chore);
        assert_eq!(prefix('R', "foo.txt"), ConventionalType::Chore);
        assert_eq!(prefix('R', "package-lock.json"), ConventionalType::Chore);
    }

    #[test]
    fn docs_are_detected() {
        assert_eq!(prefix('A', "README.md"), ConventionalType::Docs);
        assert_eq!(
            prefix('M', "docs/going/anywhere.md"),
            ConventionalType::Docs
        );
        assert_eq!(prefix('M', "notes.rst"), ConventionalType::Docs);
        assert_eq!(prefix('A', "guide.md"), ConventionalType::Docs);
        assert_eq!(prefix('M', "guide.md"), ConventionalType::Docs);
        assert_eq!(prefix('A', "RFC.markdown"), ConventionalType::Docs);
    }

    #[test]
    fn dependency_files_use_build_scope() {
        assert_eq!(
            prefix('A', "package-lock.json"),
            ConventionalType::BuildDependencies
        );
        assert_eq!(
            prefix('M', "Cargo.lock"),
            ConventionalType::BuildDependencies
        );
    }

    #[test]
    fn ci_files_are_detected() {
        assert_eq!(
            prefix('M', ".github/workflows/ci.yml"),
            ConventionalType::Ci
        );
        assert_eq!(prefix('M', ".circleci/config.yml"), ConventionalType::Ci);
    }

    #[test]
    fn build_and_config_files_are_detected() {
        assert_eq!(prefix('M', "package.json"), ConventionalType::Build);
        assert_eq!(prefix('M', "Makefile"), ConventionalType::Build);
        assert_eq!(prefix('M', ".gitignore"), ConventionalType::Chore);
        assert_eq!(prefix('M', "tsconfig.json"), ConventionalType::Chore);
    }

    #[test]
    fn tests_are_detected() {
        assert_eq!(prefix('M', "src/foo.test.ts"), ConventionalType::Test);
        assert_eq!(prefix('M', "tests/integration.rs"), ConventionalType::Test);
        assert_eq!(
            prefix('M', "spec/models/user_spec.rb"),
            ConventionalType::Test
        );
        // Configuration wins over test detection, as in the original rules.
        assert_eq!(prefix('M', ".coveragerc"), ConventionalType::Chore);
    }
}
