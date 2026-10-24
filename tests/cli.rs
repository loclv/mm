//! End-to-end tests which run the real binary against temporary Git repos.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Temporary repository which is deleted when the test finishes.
struct Repo {
    path: PathBuf,
}

impl Repo {
    /// Create an empty repository with one initial commit.
    fn new() -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mm-test-{}-{unique}", std::process::id()));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp repo");

        let repo = Self { path };

        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.email", "mm@example.com"]);
        repo.git(&["config", "user.name", "MM Test"]);
        repo.git(&["config", "commit.gpgsign", "false"]);

        repo.write("README.md", "# test\n");
        repo.git(&["add", "README.md"]);
        repo.git(&["commit", "-q", "-m", "docs: initial"]);

        repo
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Write a file relative to the repository root.
    fn write(&self, name: &str, contents: &str) {
        let path = self.path.join(name);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }

        fs::write(path, contents).expect("write test file");
    }

    /// Run Git and return trimmed stdout.
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("run git");

        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Run `mm` in the repository.
    fn mm(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mm"))
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("run mm")
    }

    /// Number of commits on the current branch.
    fn commit_count(&self) -> usize {
        self.git(&["rev-list", "--count", "HEAD"])
            .parse()
            .expect("parse commit count")
    }

    /// Subject of the latest commit.
    fn latest_subject(&self) -> String {
        self.git(&["log", "-1", "--format=%s"])
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[test]
fn prints_message_without_committing() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "feat: create foo.txt");
    assert_eq!(repo.commit_count(), 1);
}

#[test]
fn dry_run_is_an_alias_for_print() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--dry-run"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "feat: create foo.txt");
    assert_eq!(repo.commit_count(), 1);
}

#[test]
fn commits_staged_changes() {
    let repo = Repo::new();
    repo.write("src/lib.rs", "pub fn answer() -> u8 { 42 }\n");
    repo.git(&["add", "src/lib.rs"]);

    let output = repo.mm(&[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(repo.latest_subject(), "feat: create lib.rs");
    assert_eq!(repo.commit_count(), 2);
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
}

#[test]
fn falls_back_to_unstaged_tracked_changes() {
    let repo = Repo::new();
    repo.write("README.md", "# test\n\nMore docs.\n");

    let output = repo.mm(&[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(repo.latest_subject(), "docs: update README.md");
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
}

#[test]
fn staged_changes_take_priority_over_unstaged_ones() {
    let repo = Repo::new();
    repo.write("staged.txt", "staged\n");
    repo.git(&["add", "staged.txt"]);
    repo.write("README.md", "# test\n\nUnstaged docs.\n");

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "feat: create staged.txt");
}

#[test]
fn untracked_files_are_ignored_without_add() {
    let repo = Repo::new();
    repo.write("notes.txt", "hello\n");

    let output = repo.mm(&["--print"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("no changes files"));
}

#[test]
fn add_includes_untracked_files() {
    let repo = Repo::new();
    repo.write("new.rs", "fn main() {}\n");

    let output = repo.mm(&["--add"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(repo.latest_subject(), "feat: create new.rs");
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
}

#[test]
fn staged_only_refuses_to_fall_back() {
    let repo = Repo::new();
    repo.write("README.md", "# test\n\nchanged\n");

    let output = repo.mm(&["--staged", "--print"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("no changes files"));
}

#[test]
fn reports_when_there_is_nothing_to_do() {
    let repo = Repo::new();

    let output = repo.mm(&[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("Error: Unable to generate message"));
}

#[test]
fn keeps_an_existing_prefix() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print", "--message", "JIRA-1 fix: keep this"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "JIRA-1 fix: create foo.txt keep this");
}

#[test]
fn shows_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_mm"))
        .arg("--help")
        .output()
        .expect("run mm");

    assert!(output.status.success());
    assert!(stdout(&output).contains("Usage:"));
}

#[test]
fn fails_outside_a_repository() {
    let path = std::env::temp_dir().join(format!("mm-test-not-a-repo-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mm"))
        .current_dir(&path)
        .output()
        .expect("run mm");

    let _ = fs::remove_dir_all(&path);

    assert!(!output.status.success());
}

#[test]
fn handles_renames_and_moves() {
    let repo = Repo::new();
    repo.write("old/name.rs", "fn main() {}\n");
    repo.git(&["add", "old/name.rs"]);
    repo.git(&["commit", "-q", "-m", "feat: add name"]);

    fs::create_dir_all(repo.path().join("new")).unwrap();
    repo.git(&["mv", "old/name.rs", "new/name.rs"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "chore: move name.rs to new");
}

#[test]
fn verbose_mode_reports_progress() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print", "--verbose"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("[mm]"));
}

#[test]
fn honours_a_custom_type_enum() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.json",
        r#"{ "rules": { "type-enum": [2, "always", ["chore", "fix"]] } }"#,
    );
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "chore: create foo.txt");
}

#[test]
fn honours_yaml_configuration() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.yml",
        "rules:\n  type-case: [2, always, upper-case]\n  header-max-length: [2, always, 72]\n",
    );
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "FEAT: create foo.txt");
}

#[test]
fn honours_a_package_json_field() {
    let repo = Repo::new();
    repo.write(
        "package.json",
        r#"{ "name": "demo", "commitlint": { "rules": { "type-enum": [2, "always", ["wip"]] } } }"#,
    );
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "wip: create foo.txt");
}

#[test]
fn honours_header_max_length() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.json",
        r#"{ "rules": { "header-max-length": [2, "always", 20] } }"#,
    );

    for name in ["a.txt", "b.txt", "c.txt"] {
        repo.write(name, "initial\n");
        repo.git(&["add", name]);
    }
    repo.git(&["commit", "-q", "-m", "chore: add files"]);

    for name in ["a.txt", "b.txt", "c.txt"] {
        repo.write(name, "changed\n");
        repo.git(&["add", name]);
    }

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "chore: update 3 f...");
}

#[test]
fn uses_conventional_commits_for_counted_changes() {
    let repo = Repo::new();

    for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"] {
        repo.write(name, "initial\n");
        repo.git(&["add", name]);
    }
    repo.git(&["commit", "-q", "-m", "chore: add files"]);

    for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"] {
        repo.write(name, "changed\n");
        repo.git(&["add", name]);
    }

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "chore: update 5 files");
}

#[test]
fn disables_conventional_commits_with_mm_json() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "conventionalCommits": false }"#);
    repo.write("README.md", "# test\n\nMore docs.\n");

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "update README.md");
}

#[test]
fn finds_mm_json_from_a_subdirectory() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "conventionalCommits": false }"#);
    repo.write("src/deep/foo.txt", "new\n");
    repo.git(&["add", "src/deep/foo.txt"]);

    let output = Command::new(env!("CARGO_BIN_EXE_mm"))
        .args(["--print"])
        .current_dir(repo.path().join("src/deep"))
        .output()
        .expect("run mm");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "create foo.txt");
}

#[test]
fn rejects_an_invalid_mm_json_setting() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "conventionalCommits": "no" }"#);
    repo.write("README.md", "# test\n\nMore docs.\n");

    let output = repo.mm(&["--print"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("conventionalCommits"));
}

#[test]
fn drops_a_scope_which_is_not_allowed() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.json",
        r#"{ "rules": { "scope-enum": [2, "always", ["web"]] } }"#,
    );
    repo.write("package-lock.json", "{}\n");
    repo.git(&["add", "package-lock.json"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "build: create package-lock.json");
}

#[test]
fn accepts_an_explicit_config_path() {
    let repo = Repo::new();
    repo.write(
        "lint-rules.json",
        r#"{ "rules": { "type-enum": [2, "always", ["custom"]] } }"#,
    );
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print", "--config", "lint-rules.json"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "custom: create foo.txt");
}

#[test]
fn searches_parent_directories_for_configuration() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.json",
        r#"{ "rules": { "type-case": [2, "always", "upper-case"] } }"#,
    );
    repo.write("src/deep/foo.txt", "new\n");
    repo.git(&["add", "src/deep/foo.txt"]);

    let output = Command::new(env!("CARGO_BIN_EXE_mm"))
        .args(["--print"])
        .current_dir(repo.path().join("src/deep"))
        .output()
        .expect("run mm");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "FEAT: create foo.txt");
}

#[test]
fn loads_a_javascript_configuration_when_node_is_available() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }

    let repo = Repo::new();
    repo.write(
        "commitlint.config.cjs",
        "module.exports = { rules: { 'type-enum': [2, 'always', ['chore']] } };\n",
    );
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "chore: create foo.txt");
}

#[test]
fn reports_a_missing_explicit_configuration() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print", "--config", "missing.json"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("commitlint config not found"));
}
