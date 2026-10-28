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
        self.mm_with_env(args, &[])
    }

    /// Run `mm` with extra environment variables.
    fn mm_with_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        self.mm_with_env_and_remove(args, envs, &[])
    }

    /// Run `mm` with extra environment variables and specific removed variables.
    fn mm_with_env_and_remove(
        &self,
        args: &[&str],
        envs: &[(&str, &str)],
        remove_envs: &[&str],
    ) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mm"));
        command.args(args).current_dir(&self.path);

        for (key, value) in envs {
            command.env(key, value);
        }
        for key in remove_envs {
            command.env_remove(key);
        }

        command.output().expect("run mm")
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

/// Install a fake `opencode` which prints a fixed session export.
#[cfg(unix)]
fn fake_opencode(repo: &Repo, export: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let bin = repo.path().join(".fake-bin");
    fs::create_dir_all(&bin).expect("create fake bin directory");

    let fixture = repo.path().join(".chat-export.json");
    fs::write(&fixture, export).expect("write chat export");

    let script = bin.join("opencode");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s' \"$*\" > \"$MM_CHAT_ARGS\"\ncat \"$MM_CHAT_FIXTURE\"\n",
    )
    .expect("write fake opencode");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("make fake opencode executable");

    (bin, fixture)
}

#[cfg(unix)]
fn chat_env(bin: &Path, fixture: &Path, args_path: &Path) -> Vec<(String, String)> {
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    vec![
        ("PATH".to_string(), path),
        ("OPENCODE_SESSION_ID".to_string(), "ses_test".to_string()),
        ("MM_CHAT_FIXTURE".to_string(), fixture.display().to_string()),
        ("MM_CHAT_ARGS".to_string(), args_path.display().to_string()),
    ]
}

#[cfg(unix)]
fn env_refs(envs: &[(String, String)]) -> Vec<(&str, &str)> {
    envs.iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
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
    assert_eq!(repo.latest_subject(), "feat: create src/lib.rs");
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
fn honours_conventional_extends_in_commitlintrc_json() {
    let repo = Repo::new();
    repo.write(".commitlintrc.json", r#"{ "extends": ["conventional"] }"#);
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    assert_eq!(stdout(&output), "feat: create foo.txt");
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
    assert_eq!(stdout(&output), "create src/deep/foo.txt");
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
fn mm_json_auto_add_commits_untracked_files_when_nothing_is_staged() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "autoAdd": true }"#);
    repo.write("notes.txt", "hello\n");

    let output = repo.mm(&[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(repo.commit_count(), 2);
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
}

#[test]
fn mm_json_auto_add_preserves_staged_changes_when_some_are_staged() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "autoAdd": true }"#);
    repo.git(&["add", "mm.json"]);
    repo.write("staged.txt", "staged content\n");
    repo.git(&["add", "staged.txt"]);
    repo.write("untracked.txt", "still untracked\n");

    let output = repo.mm(&[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        repo.latest_subject(),
        "chore: create mm.json and staged.txt"
    );
    let status = repo.git(&["status", "--porcelain"]);
    assert!(status.contains("?? untracked.txt"), "status: {status}");
}

#[test]
fn mm_json_auto_add_respects_staged_only_flag() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "autoAdd": true }"#);
    repo.write("notes.txt", "hello\n");

    let output = repo.mm(&["--staged"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("no changes files"));
}

#[test]
fn mm_json_auto_add_disabled_by_default() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "autoAdd": false }"#);
    repo.write("notes.txt", "hello\n");

    let output = repo.mm(&[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("no changes files"));
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
    assert_eq!(stdout(&output), "FEAT: create src/deep/foo.txt");
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

#[cfg(unix)]
const CHAT_EXPORT: &str = r#"{
    "info": { "id": "ses_test", "title": "Add chat import" },
    "messages": [
        { "type": "user", "text": "Please add chat import.\nKeep the body." },
        {
            "type": "assistant",
            "content": [
                { "type": "reasoning", "text": "private reasoning" },
                { "type": "text", "text": "Implemented the import." },
                { "type": "tool", "name": "shell" }
            ]
        },
        { "type": "user", "text": "Please refine it." },
        {
            "type": "assistant",
            "content": [
                { "type": "reasoning", "text": "later reasoning" },
                { "type": "text", "text": "Final answer." },
                { "type": "tool", "name": "shell" }
            ]
        }
    ]
}"#;

#[cfg(unix)]
#[test]
fn copies_the_current_opencode_chat_with_print() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let (bin, fixture) = fake_opencode(&repo, CHAT_EXPORT);
    let args_path = repo.path().join(".chat-args");
    let envs = chat_env(&bin, &fixture, &args_path);
    let output = repo.mm_with_env(&["--chat", "--print"], &env_refs(&envs));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Add chat import\n\nFinal answer.");
    assert!(!stdout(&output).contains("private reasoning"));
    assert!(!stdout(&output).contains("later reasoning"));
    assert!(!stdout(&output).contains("Implemented the import."));
    assert_eq!(
        fs::read_to_string(&args_path).expect("read fake opencode arguments"),
        "session export ses_test"
    );
    assert_eq!(repo.commit_count(), 1);
}

#[cfg(unix)]
#[test]
fn commits_the_chat_title_and_body() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let (bin, fixture) = fake_opencode(&repo, CHAT_EXPORT);
    let args_path = repo.path().join(".chat-args");
    let envs = chat_env(&bin, &fixture, &args_path);
    let output = repo.mm_with_env(&["--chat"], &env_refs(&envs));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(repo.commit_count(), 2);
    assert_eq!(repo.latest_subject(), "Add chat import");
    assert!(
        repo.git(&["log", "-1", "--format=%b"])
            .contains("Final answer.")
    );
}

#[cfg(unix)]
#[test]
fn chat_session_implies_chat_and_falls_back_without_a_title() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let export = r#"{
        "info": {},
        "messages": [
            { "type": "user", "text": "first request" },
            { "type": "user", "text": "latest request\nwith detail" },
            { "type": "assistant", "content": [{ "type": "text", "text": "latest answer" }] }
        ]
    }"#;
    let (bin, fixture) = fake_opencode(&repo, export);
    let args_path = repo.path().join(".chat-args");
    let envs = chat_env(&bin, &fixture, &args_path);
    let output = repo.mm_with_env(
        &["--chat-session", "ses_other", "--print"],
        &env_refs(&envs),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "latest request\n\nlatest answer");
    assert_eq!(
        fs::read_to_string(&args_path).expect("read fake opencode arguments"),
        "session export ses_other"
    );
}

#[test]
fn falls_back_to_auto_commit_when_chat_session_fails() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    // Without verbose: clean output with generated commit message
    let output = repo.mm_with_env(&["--chat", "--print"], &[("OPENCODE_SESSION_ID", "")]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "feat: create foo.txt");

    // With verbose: diagnostics report the fallback
    let verbose_output = repo.mm_with_env(
        &["--chat", "--print", "--verbose"],
        &[("OPENCODE_SESSION_ID", "")],
    );
    assert!(
        verbose_output.status.success(),
        "{}",
        stderr(&verbose_output)
    );
    assert_eq!(stdout(&verbose_output), "feat: create foo.txt");
    assert!(stderr(&verbose_output).contains("failed to copy log from AI Agent"));
    assert!(stderr(&verbose_output).contains("falling back to auto commit message"));
}

#[test]
fn falls_back_to_auto_commit_when_antigravity_log_fails() {
    let repo = Repo::new();
    repo.write("bar.rs", "pub fn bar() {}\n");
    repo.git(&["add", "bar.rs"]);

    let empty_brain = std::env::temp_dir().join(format!("mm-empty-brain-{}", std::process::id()));
    fs::create_dir_all(&empty_brain).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &empty_brain.to_string_lossy())],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "feat: create bar.rs");

    let _ = fs::remove_dir_all(empty_brain);
}

#[cfg(unix)]
#[test]
fn chat_does_not_load_project_configuration() {
    let repo = Repo::new();
    repo.write("mm.json", r#"{ "conventionalCommits": "no" }"#);
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let (bin, fixture) = fake_opencode(&repo, CHAT_EXPORT);
    let args_path = repo.path().join(".chat-args");
    let envs = chat_env(&bin, &fixture, &args_path);
    let output = repo.mm_with_env(
        &["--chat", "--print", "--config", "missing.json"],
        &env_refs(&envs),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Add chat import\n\nFinal answer.");
}

#[test]
fn includes_relative_path_for_created_updated_and_deleted_files() {
    let repo = Repo::new();
    repo.write("docs/dev.md", "# Dev docs\n");
    repo.git(&["add", "docs/dev.md"]);

    let output = repo.mm(&["--print", "-m", "feat:"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "feat: create docs/dev.md");

    repo.git(&["commit", "-m", "feat: create docs/dev.md"]);
    repo.write("docs/dev.md", "# Dev docs\n\nUpdated.\n");
    repo.git(&["add", "docs/dev.md"]);

    let output = repo.mm(&["--print"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "docs: update docs/dev.md");

    repo.git(&["commit", "-m", "docs: update docs/dev.md"]);
    repo.git(&["rm", "docs/dev.md"]);

    let output = repo.mm(&["--print"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "chore: delete docs/dev.md");
}

#[test]
fn copies_antigravity_last_log_with_agy_flag() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-{}", std::process::id()));
    let conv_dir = brain.join("conv-12345");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = format!(
        r#"{{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nfix parser edge cases\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nActive Document: {}\n</ADDITIONAL_METADATA>"}}
{{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Fixed parser edge cases.\n\n- Handled trailing comma\n- Added regression test"}}
"#,
        repo.path().display()
    );
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy())],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "fix parser edge cases\n\nFixed parser edge cases.\n\n- Handled trailing comma\n- Added regression test"
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn copies_antigravity_log_with_agy_session() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-sess-{}", std::process::id()));
    let conv_dir = brain.join("my-specific-conv");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nexplicit session prompt\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Specific session answer."}
"#;
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env(
        &["--agy-session", "my-specific-conv", "--print"],
        &[
            ("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy()),
            ("ANTIGRAVITY_CONVERSATION_ID", ""),
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "explicit session prompt\n\nSpecific session answer."
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn copies_antigravity_log_with_conversation_id_env() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-env-{}", std::process::id()));
    let conv_dir = brain.join("env-conv-456");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nenv session prompt\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Env session response."}
"#;
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env(
        &["--agy", "--print"],
        &[
            ("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy()),
            ("ANTIGRAVITY_CONVERSATION_ID", "env-conv-456"),
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "env session prompt\n\nEnv session response."
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn chat_auto_detects_antigravity_when_opencode_not_set() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-auto-{}", std::process::id()));
    let conv_dir = brain.join("auto-detected-conv");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = format!(
        r#"{{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nauto detected prompt\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nActive Document: {}\n</ADDITIONAL_METADATA>"}}
{{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Auto detected response body."}}
"#,
        repo.path().display()
    );
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--chat", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy())],
        &["OPENCODE_SESSION_ID", "ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "auto detected prompt\n\nAuto detected response body."
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn copies_antigravity_log_sanitizing_user_file_urls() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-sanitize-{}", std::process::id()));
    let conv_dir = brain.join("conv-sanitize");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = format!(
        r#"{{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nfix file:///Users/secret_agent/project/foo.txt\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nActive Document: {}\n</ADDITIONAL_METADATA>"}}
{{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Updated [foo.txt](file:///Users/secret_agent/project/foo.txt).\n\nSee file:///Users/secret_agent/project/foo.txt for changes."}}
"#,
        repo.path().display()
    );
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy())],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "fix ~/project/foo.txt\n\nUpdated [foo.txt](~/project/foo.txt).\n\nSee ~/project/foo.txt for changes."
    );
    assert!(!stdout(&output).contains("secret_agent"));

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn copies_chat_log_replacing_current_repo_path_with_relative() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-repopath-{}", std::process::id()));
    let conv_dir = brain.join("conv-repopath");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let repo_path = repo.path().display().to_string();
    let repo_name = repo.path().file_name().unwrap().to_string_lossy();
    let parent_home = repo.path().parent().unwrap().to_string_lossy().to_string();

    let transcript = format!(
        r#"{{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nfix file://{repo_path}/foo.txt\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nActive Document: {repo_path}/foo.txt\n</ADDITIONAL_METADATA>"}}
{{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Updated [foo.txt](file://{repo_path}/foo.txt).\n\nSee {repo_path}/ and [tilde](~/{repo_name}/foo.txt) for details."}}
"#
    );
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[
            ("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy()),
            ("HOME", &parent_home),
        ],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "fix ./foo.txt\n\nUpdated [foo.txt](./foo.txt).\n\nSee ./ and [tilde](./foo.txt) for details."
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn copies_claude_last_log_with_claude_flag() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let claude_dir = std::env::temp_dir().join(format!("mm-test-claude-{}", std::process::id()));
    let encoded = mm::chat::encode_claude_project_path(repo.path());
    let proj_dir = claude_dir.join("projects").join(&encoded);
    fs::create_dir_all(&proj_dir).unwrap();

    let transcript = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"support claude code logs"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Added Claude Code log support."}]}}
"#;
    fs::write(proj_dir.join("sess-123.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--claude", "--print"],
        &[("CLAUDE_CONFIG_DIR", &claude_dir.to_string_lossy())],
        &["CLAUDE_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "support claude code logs\n\nAdded Claude Code log support."
    );

    let _ = fs::remove_dir_all(claude_dir);
}

#[test]
fn copies_claude_log_with_claude_session() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let claude_dir =
        std::env::temp_dir().join(format!("mm-test-claude-sess-{}", std::process::id()));
    let encoded = mm::chat::encode_claude_project_path(repo.path());
    let proj_dir = claude_dir.join("projects").join(&encoded);
    fs::create_dir_all(&proj_dir).unwrap();

    let transcript = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"fix claude session"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Fixed claude session."}]}}
"#;
    fs::write(proj_dir.join("my-specific-claude.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--claude-session", "my-specific-claude", "--print"],
        &[("CLAUDE_CONFIG_DIR", &claude_dir.to_string_lossy())],
        &["CLAUDE_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "fix claude session\n\nFixed claude session."
    );

    let _ = fs::remove_dir_all(claude_dir);
}

#[test]
fn chat_auto_detects_claude_when_others_not_set() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let claude_dir =
        std::env::temp_dir().join(format!("mm-test-claude-auto-{}", std::process::id()));
    let encoded = mm::chat::encode_claude_project_path(repo.path());
    let proj_dir = claude_dir.join("projects").join(&encoded);
    fs::create_dir_all(&proj_dir).unwrap();

    let transcript = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"auto detected claude prompt"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Auto detected claude response."}]}}
"#;
    fs::write(proj_dir.join("auto-sess.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--chat", "--print"],
        &[("CLAUDE_CONFIG_DIR", &claude_dir.to_string_lossy())],
        &[
            "OPENCODE_SESSION_ID",
            "ANTIGRAVITY_CONVERSATION_ID",
            "CLAUDE_SESSION_ID",
            "CODEX_SESSION_ID",
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "auto detected claude prompt\n\nAuto detected claude response."
    );

    let _ = fs::remove_dir_all(claude_dir);
}

#[test]
fn copies_codex_last_log_with_codex_flag() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let codex_dir = std::env::temp_dir().join(format!("mm-test-codex-{}", std::process::id()));
    let sessions_dir = codex_dir
        .join("sessions")
        .join("2026")
        .join("10")
        .join("27");
    fs::create_dir_all(&sessions_dir).unwrap();

    let transcript = format!(
        r#"{{"type":"session_meta","payload":{{"id":"019e274d-bfd2-7093-9ec4-13c38e1c66e7","cwd":"{}"}}}}
{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"support codex code logs"}}]}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"Added OpenAI Codex log support."}}]}}}}
"#,
        repo.path().to_string_lossy()
    );
    fs::write(
        sessions_dir.join("rollout-2026-10-27T00-00-00-019e274d-bfd2-7093-9ec4-13c38e1c66e7.jsonl"),
        transcript,
    )
    .unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--codex", "--print"],
        &[("CODEX_HOME", &codex_dir.to_string_lossy())],
        &["CODEX_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "support codex code logs\n\nAdded OpenAI Codex log support."
    );

    let _ = fs::remove_dir_all(codex_dir);
}

#[test]
fn copies_codex_log_with_codex_session() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let codex_dir = std::env::temp_dir().join(format!("mm-test-codex-sess-{}", std::process::id()));
    let sessions_dir = codex_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();

    let transcript = r#"{"type":"session_meta","payload":{"id":"my-specific-codex","cwd":"/some/dir"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fix codex session"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Fixed codex session."}]}}
"#;
    fs::write(
        sessions_dir.join("rollout-my-specific-codex.jsonl"),
        transcript,
    )
    .unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--codex-session", "my-specific-codex", "--print"],
        &[("CODEX_HOME", &codex_dir.to_string_lossy())],
        &["CODEX_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "fix codex session\n\nFixed codex session.");

    let _ = fs::remove_dir_all(codex_dir);
}

#[test]
fn chat_auto_detects_codex_when_others_not_set() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let codex_dir = std::env::temp_dir().join(format!("mm-test-codex-auto-{}", std::process::id()));
    let sessions_dir = codex_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();

    let transcript = format!(
        r#"{{"type":"session_meta","payload":{{"id":"auto-codex-sess","cwd":"{}"}}}}
{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"auto detected codex prompt"}}]}}}}
{{"type":"event_msg","payload":{{"type":"task_complete","last_agent_message":"Auto detected codex response."}}}}
"#,
        repo.path().to_string_lossy()
    );
    fs::write(sessions_dir.join("rollout-auto-codex.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--chat", "--print"],
        &[("CODEX_HOME", &codex_dir.to_string_lossy())],
        &[
            "OPENCODE_SESSION_ID",
            "ANTIGRAVITY_CONVERSATION_ID",
            "CLAUDE_SESSION_ID",
            "CODEX_SESSION_ID",
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "auto detected codex prompt\n\nAuto detected codex response."
    );

    let _ = fs::remove_dir_all(codex_dir);
}

#[test]
fn copies_codex_session_with_title_from_session_index() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let codex_dir =
        std::env::temp_dir().join(format!("mm-test-codex-index-{}", std::process::id()));
    let sessions_dir = codex_dir.join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();

    let index_content = r#"{"id":"01a10459-bbd6-7f61-97ea-12a1ac1a7e91","thread_name":"Session Index Contributor Guide","updated_at":"2026-10-04T00:39:24.690019Z"}
"#;
    fs::write(codex_dir.join("session_index.jsonl"), index_content).unwrap();

    let transcript = format!(
        r#"{{"type":"session_meta","payload":{{"id":"01a10459-bbd6-7f61-97ea-12a1ac1a7e91","cwd":"{}"}}}}
{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"Some prompt line"}}]}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"Created contributor guide."}}]}}}}
"#,
        repo.path().to_string_lossy()
    );
    fs::write(
        sessions_dir.join("rollout-01a10459-bbd6-7f61-97ea-12a1ac1a7e91.jsonl"),
        transcript,
    )
    .unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--codex", "--print"],
        &[("CODEX_HOME", &codex_dir.to_string_lossy())],
        &["CODEX_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Session Index Contributor Guide\n\nCreated contributor guide."
    );

    let _ = fs::remove_dir_all(codex_dir);
}

#[test]
fn copies_devin_last_log_with_devin_flag() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let devin_dir = std::env::temp_dir().join(format!("mm-test-devin-{}", std::process::id()));
    let transcripts_dir = devin_dir.join("transcripts");
    fs::create_dir_all(&transcripts_dir).unwrap();

    let transcript = r#"{
  "schema_version": "ATIF-v1.7",
  "session_id": "session-devin-1",
  "steps": [
    {
      "role": "user",
      "content": "implement Devin log parser"
    },
    {
      "role": "assistant",
      "content": "Added Devin support."
    }
  ]
}"#;
    fs::write(transcripts_dir.join("devin-1.json"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--devin", "--print"],
        &[("DEVIN_HOME", &devin_dir.to_string_lossy())],
        &["DEVIN_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "implement Devin log parser\n\nAdded Devin support."
    );

    let _ = fs::remove_dir_all(devin_dir);
}

#[test]
fn copies_devin_log_with_devin_session() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let devin_dir = std::env::temp_dir().join(format!("mm-test-devin-sess-{}", std::process::id()));
    let transcripts_dir = devin_dir.join("transcripts");
    fs::create_dir_all(&transcripts_dir).unwrap();

    let transcript = r#"{"role":"user","content":"fix devin session"}
{"role":"assistant","content":"Fixed devin session."}
"#;
    fs::write(transcripts_dir.join("my-specific-devin.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--devin-session", "my-specific-devin", "--print"],
        &[("DEVIN_HOME", &devin_dir.to_string_lossy())],
        &["DEVIN_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "fix devin session\n\nFixed devin session.");

    let _ = fs::remove_dir_all(devin_dir);
}

#[test]
fn copies_cascade_markdown_trajectory_with_codeium_flag() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let devin_dir = std::env::temp_dir().join(format!("mm-test-cascade-{}", std::process::id()));
    let cascade_dir = devin_dir.join("cascade");
    fs::create_dir_all(&cascade_dir).unwrap();

    let trajectory = r#"# Cascade Trajectory

**User**
cascade user request

**Assistant**
Thinking Process:
- analyzing request

Trajectory executed successfully.
"#;
    fs::write(cascade_dir.join("trajectory.md"), trajectory).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--codeium", "--print"],
        &[("DEVIN_HOME", &devin_dir.to_string_lossy())],
        &["DEVIN_SESSION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "cascade user request\n\nTrajectory executed successfully."
    );

    let _ = fs::remove_dir_all(devin_dir);
}

#[test]
fn chat_auto_detects_devin_when_others_not_set() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let devin_dir = std::env::temp_dir().join(format!("mm-test-devin-auto-{}", std::process::id()));
    let transcripts_dir = devin_dir.join("transcripts");
    fs::create_dir_all(&transcripts_dir).unwrap();

    let transcript = format!(
        r#"{{"role":"user","content":"auto detected devin in {}"}}
{{"role":"assistant","content":"Auto detected devin response."}}
"#,
        repo.path().to_string_lossy()
    );
    fs::write(transcripts_dir.join("auto-devin.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--chat", "--print"],
        &[("DEVIN_HOME", &devin_dir.to_string_lossy())],
        &[
            "DEVIN_SESSION_ID",
            "OPENCODE_SESSION_ID",
            "ANTIGRAVITY_CONVERSATION_ID",
            "CLAUDE_SESSION_ID",
            "CODEX_SESSION_ID",
        ],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "auto detected devin in ./\n\nAuto detected devin response."
    );

    let _ = fs::remove_dir_all(devin_dir);
}

#[test]
fn chat_auto_adds_commitlint_prefix_when_commitlint_is_configured() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.json",
        r#"{
  "extends": ["@commitlint/config-conventional"],
  "rules": {
    "header-max-length": [2, "always", 120]
  }
}"#,
    );
    repo.write("src/lib.rs", "pub fn hello() {}\n");
    repo.git(&["add", "src/lib.rs"]);

    let brain =
        std::env::temp_dir().join(format!("mm-test-brain-commitlint-{}", std::process::id()));
    let conv_dir = brain.join("conv-commitlint");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let repo_path = repo.path().display().to_string();
    let transcript = format!(
        r#"{{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nadd feature in file://{repo_path}/src/lib.rs...\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nActive Document: {repo_path}/src/lib.rs\n</ADDITIONAL_METADATA>"}}
{{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Done."}}
"#
    );
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy())],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "feat: add feature in ./src/lib.rs\n\nDone."
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn chat_uses_docs_prefix_when_all_changes_are_markdown_even_with_feat_keywords() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.json",
        r#"{
  "extends": ["@commitlint/config-conventional"]
}"#,
    );
    repo.write("README.md", "# updated docs\n");
    repo.git(&["add", "README.md"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-docs-md-{}", std::process::id()));
    let conv_dir = brain.join("conv-docs-md");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nadd option at \"mm.json\" file that auto commit with only `mm` CLI\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Added the option."}
"#;
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy())],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "docs: add option at \"mm.json\" file that auto commit with only `mm` CLI\n\nAdded the option."
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn chat_infers_feat_prefix_when_log_includes_feat_keywords_and_code_changes() {
    let repo = Repo::new();
    repo.write(
        ".commitlintrc.json",
        r#"{
  "extends": ["@commitlint/config-conventional"]
}"#,
    );
    repo.write("src/cli.rs", "// modified\n");
    repo.git(&["add", "src/cli.rs"]);

    let brain =
        std::env::temp_dir().join(format!("mm-test-brain-feat-keyword-{}", std::process::id()));
    let conv_dir = brain.join("conv-feat-keyword");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nadd option at \"mm.json\" file that auto commit with only `mm` CLI\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Added the option."}
"#;
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy())],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "feat: add option at \"mm.json\" file that auto commit with only `mm` CLI\n\nAdded the option."
    );

    let _ = fs::remove_dir_all(brain);
}

#[test]
fn all_markdown_files_use_docs_prefix() {
    let repo = Repo::new();
    repo.write("guide.md", "# Guide\n");
    repo.write("deploy.markdown", "# Deploy\n");
    repo.git(&["add", "guide.md", "deploy.markdown"]);

    let output = repo.mm(&["--print"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "docs: create deploy.markdown and guide.md");
}

#[test]
fn all_markdown_files_created_updated_deleted_use_docs_prefix() {
    let repo = Repo::new();
    repo.write("old.md", "# Old\n");
    repo.write("edit.md", "# Edit\n");
    repo.git(&["add", "old.md", "edit.md"]);
    repo.git(&["commit", "-m", "docs: initial docs"]);

    repo.write("edit.md", "# Edit updated\n");
    repo.write("created.md", "# Created\n");
    repo.git(&["rm", "old.md"]);
    repo.git(&["add", "edit.md", "created.md"]);

    let output = repo.mm(&["--print"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "docs: create 1 file, update 1 file and delete 1 file"
    );
}

#[test]
fn copies_chat_log_sanitizing_filler_tool_calls_and_ansi() {
    let repo = Repo::new();
    repo.write("foo.txt", "new\n");
    repo.git(&["add", "foo.txt"]);

    let brain = std::env::temp_dir().join(format!("mm-test-brain-sanitize-{}", std::process::id()));
    let conv_dir = brain.join("conv-sanitize");
    let logs_dir = conv_dir.join(".system_generated/logs");
    fs::create_dir_all(&logs_dir).unwrap();

    let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nimplement smart body sanitization\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"\u001b[32mSure! I will help you with that.\u001b[0m\n\n<tool_call>\n{\"name\": \"run_tests\"}\n</tool_call>\n[Tool Output: all passed]\n\nHere is a summary:\n- Stripped ANSI escapes\n- Stripped tool call residue\n- Stripped conversational filler\n\nHope this helps! Let me know if you need anything else."}
"#;
    fs::write(logs_dir.join("transcript.jsonl"), transcript).unwrap();

    let output = repo.mm_with_env_and_remove(
        &["--agy", "--print"],
        &[("ANTIGRAVITY_BRAIN_DIR", &brain.to_string_lossy())],
        &["ANTIGRAVITY_CONVERSATION_ID"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "implement smart body sanitization\n\n- Stripped ANSI escapes\n- Stripped tool call residue\n- Stripped conversational filler"
    );

    let _ = fs::remove_dir_all(brain);
}
