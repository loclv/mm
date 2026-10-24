//! Read the project's `mm.json` configuration.
//!
//! The only setting currently supported is `conventionalCommits`. It defaults
//! to `true`, so generated messages follow the Conventional Commits
//! specification unless a project explicitly opts out.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Name of the configuration file, searched for from the current directory.
pub const FILE_NAME: &str = "mm.json";

/// Configuration loaded from `mm.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Whether generated messages use the Conventional Commits format.
    pub conventional_commits: bool,
    /// File the configuration came from, if any.
    pub source: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            conventional_commits: true,
            source: None,
        }
    }
}

/// Load the nearest `mm.json`, or return the defaults when there is none.
///
/// Directories are searched from `start` upwards, stopping at the home
/// directory like the commitlint search.
pub fn load(start: &Path) -> Result<Config, String> {
    let Some(path) = find(start) else {
        return Ok(Config::default());
    };

    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{} must contain a JSON object", path.display()))?;

    let conventional_commits = read_conventional_commits(object, &path)?;

    Ok(Config {
        conventional_commits,
        source: Some(path),
    })
}

/// Find `mm.json` in `start` or one of its ancestors.
pub fn find(start: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    for directory in start.ancestors() {
        let path = directory.join(FILE_NAME);

        if path.is_file() {
            return Some(path);
        }

        if home.as_deref() == Some(directory) {
            break;
        }
    }

    None
}

/// Read the Conventional Commits switch, accepting a short alias.
fn read_conventional_commits(
    object: &serde_json::Map<String, Value>,
    path: &Path,
) -> Result<bool, String> {
    for key in ["conventionalCommits", "conventional"] {
        let Some(value) = object.get(key) else {
            continue;
        };

        return match value {
            Value::Bool(value) => Ok(*value),
            _ => Err(format!("{}: `{key}` must be a boolean", path.display())),
        };
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mm-config-{}-{unique}", std::process::id()));

            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp directory");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write(&self, name: &str, contents: &str) {
            let path = self.path.join(name);

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }

            fs::write(path, contents).expect("write config");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn defaults_to_conventional_commits() {
        let dir = TempDir::new();
        let config = load(dir.path()).unwrap();

        assert!(config.conventional_commits);
        assert_eq!(config.source, None);
    }

    #[test]
    fn reads_a_disabled_conventional_commits_setting() {
        let dir = TempDir::new();
        dir.write(FILE_NAME, r#"{ "conventionalCommits": false }"#);

        let config = load(dir.path()).unwrap();

        assert!(!config.conventional_commits);
        assert_eq!(config.source, Some(dir.path().join(FILE_NAME)));
    }

    #[test]
    fn accepts_the_short_alias() {
        let dir = TempDir::new();
        dir.write(FILE_NAME, r#"{ "conventional": false }"#);

        assert!(!load(dir.path()).unwrap().conventional_commits);
    }

    #[test]
    fn searches_parent_directories() {
        let dir = TempDir::new();
        let nested = dir.path().join("src/deep");
        fs::create_dir_all(&nested).unwrap();
        dir.write(FILE_NAME, r#"{ "conventionalCommits": false }"#);

        assert!(!load(&nested).unwrap().conventional_commits);
    }

    #[test]
    fn rejects_a_non_boolean_setting() {
        let dir = TempDir::new();
        dir.write(FILE_NAME, r#"{ "conventionalCommits": "no" }"#);

        let error = load(dir.path()).unwrap_err();

        assert!(error.contains("conventionalCommits"));
        assert!(error.contains("boolean"));
    }

    #[test]
    fn rejects_invalid_json() {
        let dir = TempDir::new();
        dir.write(FILE_NAME, "{");

        assert!(load(dir.path()).is_err());
    }
}
