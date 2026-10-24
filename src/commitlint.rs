//! Read a project's commitlint configuration.
//!
//! commitlint is resolved by `@commitlint/cli` using cosmiconfig, which supports
//! every file in [`SEARCH_PLACES`] plus a `commitlint` field in `package.json`
//! or `package.yaml`. JavaScript and TypeScript configs must be evaluated by
//! Node, so those are loaded through `@commitlint/load` (which also resolves
//! `extends` presets). JSON, YAML and `package.json` configs are parsed natively
//! and only call Node when they use `extends`.
//!
//! The result is a [`Policy`] containing the rules which affect message
//! generation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

use crate::policy::{Case, Policy};

/// Files checked in each directory, in the same order as commitlint.
pub const SEARCH_PLACES: &[&str] = &[
    "package.json",
    "package.yaml",
    ".commitlintrc",
    ".commitlintrc.json",
    ".commitlintrc.yaml",
    ".commitlintrc.yml",
    ".commitlintrc.js",
    ".commitlintrc.cjs",
    ".commitlintrc.mjs",
    ".commitlintrc.ts",
    ".commitlintrc.cts",
    ".commitlintrc.mts",
    "commitlint.config.js",
    "commitlint.config.cjs",
    "commitlint.config.mjs",
    "commitlint.config.ts",
    "commitlint.config.cts",
    "commitlint.config.mts",
];

/// A loaded commitlint configuration.
#[derive(Debug, Clone)]
pub struct Commitlint {
    /// File the configuration came from.
    pub source: PathBuf,
    /// How it was loaded: `commitlint`, `direct` or `file`.
    pub via: &'static str,
    pub policy: Policy,
    pub warnings: Vec<String>,
}

/// Config file formats supported by commitlint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    PackageJson,
    PackageYaml,
    Json,
    Yaml,
    Javascript,
    Typescript,
    /// `.commitlintrc` without an extension: JSON or YAML.
    Unset,
}

impl Kind {
    fn of(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        match name.as_str() {
            "package.json" => Self::PackageJson,
            "package.yaml" | "package.yml" => Self::PackageYaml,
            _ => match extension.as_str() {
                "json" => Self::Json,
                "yaml" | "yml" => Self::Yaml,
                "js" | "cjs" | "mjs" => Self::Javascript,
                "ts" | "cts" | "mts" => Self::Typescript,
                _ => Self::Unset,
            },
        }
    }

    fn is_javascript(self) -> bool {
        matches!(self, Self::Javascript | Self::Typescript)
    }

    fn is_package(self) -> bool {
        matches!(self, Self::PackageJson | Self::PackageYaml)
    }
}

/// Load the nearest commitlint configuration, if there is one.
///
/// `explicit` comes from `--config` and is an error when it cannot be read.
/// Without it, directories are searched from `start` upwards, stopping at the
/// home directory like commitlint does.
pub fn load(start: &Path, explicit: Option<&Path>) -> Result<Option<Commitlint>, String> {
    let path = match explicit {
        Some(path) => {
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                start.join(path)
            };

            if !path.is_file() {
                return Err(format!("commitlint config not found: {}", path.display()));
            }

            path
        }
        None => match find_config(start) {
            Some(path) => path,
            None => return Ok(None),
        },
    };

    let kind = Kind::of(&path);
    let native = match kind {
        Kind::Javascript | Kind::Typescript => None,
        _ => Some(read_native(&path, kind)?),
    };

    let extends = native.as_ref().map(extends_of).unwrap_or_default();
    let mut warnings = Vec::new();

    if kind.is_javascript() || !extends.is_empty() {
        match load_with_node(&path, kind) {
            Ok(loaded) => {
                let policy = policy_from_rules(&loaded.rules)?;

                if loaded.via == "direct" && !loaded.extends.is_empty() {
                    warnings.push(format!(
                        "`extends` was not resolved because @commitlint/load is not available: {}",
                        loaded.extends.join(", ")
                    ));
                }
                if let Some(warning) = loaded.warning {
                    warnings.push(warning);
                }

                return Ok(Some(Commitlint {
                    source: path,
                    via: loaded.via,
                    policy,
                    warnings,
                }));
            }
            Err(error) => {
                if native.is_none() {
                    return Err(format!(
                        "could not load commitlint config {}: {error}",
                        path.display()
                    ));
                }

                warnings.push(format!(
                    "`extends` was not resolved because @commitlint/load could not run: {error}"
                ));
            }
        }
    }

    let config = native.unwrap_or(Value::Null);
    let policy = policy_from_rules(rules_of(&config))?;

    Ok(Some(Commitlint {
        source: path,
        via: "file",
        policy,
        warnings,
    }))
}

/// Search upwards for a commitlint config, stopping at the home directory.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    for directory in start.ancestors() {
        for name in SEARCH_PLACES {
            let path = directory.join(name);

            if !path.is_file() {
                continue;
            }

            let found = match *name {
                "package.json" | "package.yaml" | "package.yml" => package_has_commitlint(&path),
                _ => true,
            };

            if found {
                return Some(path);
            }
        }

        if home.as_deref() == Some(directory) {
            break;
        }
    }

    None
}

/// Check whether `package.json` or `package.yaml` has a `commitlint` field.
fn package_has_commitlint(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };

    let value = match Kind::of(path) {
        Kind::PackageJson => serde_json::from_str::<Value>(&text).ok(),
        Kind::PackageYaml => yaml_to_json(path, &text).ok(),
        _ => None,
    };

    value.is_some_and(|value| value.get("commitlint").is_some())
}

/// Read and parse a JSON, YAML or package manifest config.
fn read_native(path: &Path, kind: Kind) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;

    match kind {
        Kind::PackageJson => {
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))?;

            Ok(value.get("commitlint").cloned().unwrap_or(Value::Null))
        }
        Kind::PackageYaml => {
            let value = yaml_to_json(path, &text)?;

            Ok(value.get("commitlint").cloned().unwrap_or(Value::Null))
        }
        Kind::Json => serde_json::from_str(&text)
            .map_err(|error| format!("invalid JSON in {}: {error}", path.display())),
        Kind::Yaml => yaml_to_json(path, &text),
        Kind::Unset => serde_json::from_str(&text).or_else(|_| yaml_to_json(path, &text)),
        Kind::Javascript | Kind::Typescript => Err("JavaScript config needs Node".to_string()),
    }
}

/// Parse YAML and normalise it to JSON values.
fn yaml_to_json(path: &Path, text: &str) -> Result<Value, String> {
    let value: serde_norway::Value = serde_norway::from_str(text)
        .map_err(|error| format!("invalid YAML in {}: {error}", path.display()))?;

    serde_json::to_value(value)
        .map_err(|error| format!("could not read YAML in {}: {error}", path.display()))
}

/// List values from the `extends` key.
fn extends_of(config: &Value) -> Vec<String> {
    match config.get("extends") {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract the `rules` object from a configuration.
///
/// Files may contain the rules directly (for example a `.commitlintrc` with
/// only rule names), so that shape is accepted too.
fn rules_of(config: &Value) -> &Value {
    let Some(object) = config.as_object() else {
        return &Value::Null;
    };

    if let Some(rules) = object.get("rules") {
        return rules;
    }

    if object.keys().any(|key| key.contains('-')) {
        return config;
    }

    &Value::Null
}

/// Convert commitlint rules into a generation policy.
pub fn policy_from_rules(rules: &Value) -> Result<Policy, String> {
    let mut policy = Policy::default();

    let Some(object) = rules.as_object() else {
        return Ok(policy);
    };

    for (name, entry) in object {
        let Some(rule) = Rule::read(entry) else {
            continue;
        };

        match name.as_str() {
            "type-enum" => match rule.condition {
                "always" => policy.types = rule.strings(name),
                "never" => policy.deny_types = rule.strings(name),
                _ => {}
            },
            "type-case" => {
                policy.type_case = rule.string_value().and_then(Case::parse);
            }
            "type-empty" => match rule.condition {
                "never" => policy.type_required = true,
                "always" => policy.type_forbidden = true,
                _ => {}
            },
            "type-max-length" => policy.type_max_length = rule.number(),
            "scope-enum" => match rule.condition {
                "always" => policy.scopes = rule.scopes(name),
                "never" => policy.deny_scopes = rule.scopes(name),
                _ => {}
            },
            "scope-case" => {
                policy.scope_case = rule.case_value();
            }
            "scope-empty" => match rule.condition {
                "always" => policy.scope_forbidden = true,
                "never" => policy.scope_required = true,
                _ => {}
            },
            "header-max-length" => policy.header_max_length = rule.number(),
            "subject-max-length" => policy.subject_max_length = rule.number(),
            "header-full-stop" | "subject-full-stop" if rule.condition == "never" => {
                policy.strip_full_stop = Some(rule.string_value().unwrap_or(".").to_string());
            }
            _ => {}
        }
    }

    Ok(policy)
}

/// One enabled rule from a commitlint configuration.
struct Rule<'a> {
    condition: &'a str,
    value: Option<&'a Value>,
}

impl<'a> Rule<'a> {
    /// Parse `[severity, condition, value?]`, ignoring disabled rules.
    fn read(entry: &'a Value) -> Option<Self> {
        let array = entry.as_array()?;
        let level = array.first()?.as_u64()?;

        if level == 0 {
            return None;
        }

        Some(Self {
            condition: array.get(1)?.as_str()?,
            value: array.get(2),
        })
    }

    fn string_value(&self) -> Option<&'a str> {
        self.value?.as_str()
    }

    fn number(&self) -> Option<usize> {
        self.value?.as_u64().map(|value| value as usize)
    }

    fn strings(&self, rule: &str) -> Option<Vec<String>> {
        let values = self.value?.as_array()?;
        let strings: Vec<String> = values
            .iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect();

        if strings.is_empty() {
            return None;
        }

        if strings.len() != values.len() {
            eprintln!("warning: ignoring non-string values in commitlint rule `{rule}`");
        }

        Some(strings)
    }

    /// Read `scope-enum`, which may also be an object with a `scopes` key.
    fn scopes(&self, rule: &str) -> Option<Vec<String>> {
        let value = self.value?;

        if let Some(object) = value.as_object() {
            let values = object.get("scopes")?.as_array()?;

            return Some(
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(ToString::to_string))
                    .collect(),
            );
        }

        self.strings(rule)
    }

    /// Read `scope-case`, which may be an object with a `cases` key.
    fn case_value(&self) -> Option<Case> {
        let value = self.value?;

        if let Some(object) = value.as_object() {
            return object
                .get("cases")?
                .as_array()?
                .iter()
                .filter_map(|value| value.as_str())
                .find_map(Case::parse);
        }

        value.as_str().and_then(Case::parse)
    }
}

/// Output of the Node loader.
struct NodeConfig {
    rules: Value,
    extends: Vec<String>,
    via: &'static str,
    warning: Option<String>,
}

/// JavaScript evaluated by Node.
///
/// `@commitlint/load` is used when installed so that `extends` presets are
/// resolved with commitlint's own resolution rules. If it is missing, the config
/// file is imported directly and only its local rules are used.
const NODE_LOADER: &str = r#"
const path = require('node:path');
const { createRequire } = require('node:module');
const { pathToFileURL } = require('node:url');

const cwd = process.argv[1];
const file = process.argv[2] || '';
const req = createRequire(path.join(cwd, '__ac_loader__.js'));

function emit(rules, extra) {
  process.stdout.write(JSON.stringify(Object.assign({ rules: rules || {} }, extra)));
}

(async () => {
  let load = null;

  try {
    const mod = req('@commitlint/load');
    load = mod.load || (mod.default && (mod.default.load || mod.default)) || null;

    if (typeof load !== 'function') {
      load = null;
    }
  } catch (error) {
    load = null;
  }

  if (load) {
    try {
      const options = file
        ? { cwd: path.dirname(file), file }
        : { cwd, resolveFrom: cwd };
      const result = await load({}, options);
      const config = result.config || {};
      const extended = Array.isArray(config.extends)
        ? config.extends
        : (config.extends ? [config.extends] : []);

      emit(result.rules, { extends: extended, via: 'commitlint' });
      return;
    } catch (error) {
      if (!file) {
        throw error;
      }

      var loadError = error;
    }
  }

  if (!file) {
    throw new Error('@commitlint/load is not installed');
  }

  const url = pathToFileURL(file).href;
  const mod = await import(url);
  const config = (mod && (mod.default || mod)) || {};
  const extended = Array.isArray(config.extends)
    ? config.extends
    : (config.extends ? [config.extends] : []);

  emit(config.rules, {
    extends: extended,
    via: 'direct',
    warning: loadError ? String((loadError && loadError.message) || loadError) : undefined,
  });
})().catch((error) => {
  console.error(error && (error.stack || error.message) || String(error));
  process.exit(1);
});
"#;

/// Load a config by running Node in the config's directory.
fn load_with_node(path: &Path, kind: Kind) -> Result<NodeConfig, String> {
    let directory = path.parent().unwrap_or(Path::new("."));
    let directory_text = directory.to_string_lossy().into_owned();
    let file = if kind.is_package() {
        String::new()
    } else {
        path.to_string_lossy().into_owned()
    };

    let mut output = run_node(directory, &directory_text, &file, &[])?;

    if !output.status.success() && kind == Kind::Typescript {
        output = run_node(
            directory,
            &directory_text,
            &file,
            &["--experimental-strip-types"],
        )?;
    }

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();

        return Err(if detail.is_empty() {
            "node could not load the configuration".to_string()
        } else {
            detail
        });
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("unexpected output from Node: {error}"))?;

    Ok(NodeConfig {
        rules: value.get("rules").cloned().unwrap_or(Value::Null),
        extends: value
            .get("extends")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        via: match value.get("via").and_then(Value::as_str) {
            Some("commitlint") => "commitlint",
            _ => "direct",
        },
        warning: value
            .get("warning")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

/// Run the Node loader.
fn run_node(
    directory: &Path,
    directory_text: &str,
    file: &str,
    extra_args: &[&str],
) -> Result<Output, String> {
    Command::new("node")
        .args(extra_args)
        .arg("--no-warnings")
        .arg("-e")
        .arg(NODE_LOADER)
        .arg(directory_text)
        .arg(file)
        .current_dir(directory)
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => "node is not installed".to_string(),
            _ => format!("could not run node: {error}"),
        })
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
                std::env::temp_dir().join(format!("mm-commitlint-{}-{unique}", std::process::id()));

            fs::create_dir_all(&path).unwrap();

            Self { path }
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn rules(json: &str) -> Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn reads_rules_into_a_policy() {
        let policy = policy_from_rules(&rules(
            r#"{
                "type-enum": [2, "always", ["feat", "fix", "chore"]],
                "type-case": [2, "always", "upper-case"],
                "type-empty": [2, "never"],
                "type-max-length": [2, "always", 10],
                "scope-enum": [2, "always", ["deps", "api"]],
                "scope-case": [2, "always", "lower-case"],
                "header-max-length": [2, "always", 72],
                "subject-max-length": [2, "always", 50],
                "subject-full-stop": [2, "never", "."]
            }"#,
        ))
        .unwrap();

        assert_eq!(
            policy.types,
            Some(vec![
                "feat".to_string(),
                "fix".to_string(),
                "chore".to_string()
            ])
        );
        assert_eq!(policy.type_case, Some(Case::Upper));
        assert!(policy.type_required);
        assert_eq!(policy.type_max_length, Some(10));
        assert_eq!(
            policy.scopes,
            Some(vec!["deps".to_string(), "api".to_string()])
        );
        assert_eq!(policy.scope_case, Some(Case::Lower));
        assert_eq!(policy.header_max_length, Some(72));
        assert_eq!(policy.subject_max_length, Some(50));
        assert_eq!(policy.strip_full_stop, Some(".".to_string()));
    }

    #[test]
    fn ignores_disabled_and_unknown_rules() {
        let policy = policy_from_rules(&rules(
            r#"{
                "type-enum": [0, "always", ["feat"]],
                "scope-empty": [2, "always"],
                "made-up-rule": [2, "always", true]
            }"#,
        ))
        .unwrap();

        assert_eq!(policy.types, None);
        assert!(policy.scope_forbidden);
    }

    #[test]
    fn supports_scope_objects_and_never_enums() {
        let policy = policy_from_rules(&rules(
            r#"{
                "scope-enum": [2, "always", { "scopes": ["api", "web"] }],
                "scope-case": [2, "always", { "cases": ["kebab-case"] }],
                "type-enum": [2, "never", ["wip"]]
            }"#,
        ))
        .unwrap();

        assert_eq!(
            policy.scopes,
            Some(vec!["api".to_string(), "web".to_string()])
        );
        assert_eq!(policy.scope_case, Some(Case::Kebab));
        assert_eq!(policy.deny_types, Some(vec!["wip".to_string()]));
    }

    #[test]
    fn handles_infinity_lengths_as_missing() {
        let policy =
            policy_from_rules(&rules(r#"{ "header-max-length": [2, "always", null] }"#)).unwrap();

        assert_eq!(policy.header_max_length, None);
    }

    #[test]
    fn reads_json_config() {
        let dir = TempDir::new();
        let path = dir.write(
            ".commitlintrc.json",
            r#"{ "rules": { "header-max-length": [2, "always", 60] } }"#,
        );

        let config = load(&dir.path, Some(&path)).unwrap().unwrap();

        assert_eq!(config.policy.header_max_length, Some(60));
        assert_eq!(config.via, "file");
    }

    #[test]
    fn reads_yaml_config() {
        let dir = TempDir::new();
        let path = dir.write(
            ".commitlintrc.yaml",
            "rules:\n  type-enum:\n    - 2\n    - always\n    - [wip, feat]\n",
        );

        let config = load(&dir.path, Some(&path)).unwrap().unwrap();

        assert_eq!(
            config.policy.types,
            Some(vec!["wip".to_string(), "feat".to_string()])
        );
    }

    #[test]
    fn reads_extensionless_yaml_config() {
        let dir = TempDir::new();
        dir.write(
            ".commitlintrc",
            "rules:\n  header-max-length: [2, always, 40]\n",
        );

        let config = load(&dir.path, None).unwrap().unwrap();

        assert_eq!(config.policy.header_max_length, Some(40));
    }

    #[test]
    fn reads_package_json_field() {
        let dir = TempDir::new();
        dir.write(
            "package.json",
            r#"{ "name": "demo", "commitlint": { "rules": { "type-enum": [2, "always", ["feat"]] } } }"#,
        );

        let config = load(&dir.path, None).unwrap().unwrap();

        assert_eq!(config.policy.types, Some(vec!["feat".to_string()]));
        assert_eq!(config.source.file_name().unwrap(), "package.json");
    }

    #[test]
    fn ignores_package_json_without_a_commitlint_field() {
        let dir = TempDir::new();
        dir.write("package.json", r#"{ "name": "demo" }"#);

        assert!(load(&dir.path, None).unwrap().is_none());
    }

    #[test]
    fn reads_package_yaml_field() {
        let dir = TempDir::new();
        dir.write(
            "package.yaml",
            "name: demo\ncommitlint:\n  rules:\n    header-max-length: [2, always, 30]\n",
        );

        let config = load(&dir.path, None).unwrap().unwrap();

        assert_eq!(config.policy.header_max_length, Some(30));
    }

    #[test]
    fn prefers_a_config_file_over_package_json_without_field() {
        let dir = TempDir::new();
        dir.write("package.json", r#"{ "name": "demo" }"#);
        dir.write(
            ".commitlintrc.json",
            r#"{ "rules": { "header-max-length": [2, "always", 20] } }"#,
        );

        let config = load(&dir.path, None).unwrap().unwrap();

        assert_eq!(config.policy.header_max_length, Some(20));
        assert_eq!(config.source.file_name().unwrap(), ".commitlintrc.json");
    }

    #[test]
    fn searches_parent_directories() {
        let dir = TempDir::new();
        dir.write(
            ".commitlintrc.json",
            r#"{ "rules": { "header-max-length": [2, "always", 50] } }"#,
        );

        let nested = dir.path.join("src/deep");
        fs::create_dir_all(&nested).unwrap();

        let config = load(&nested, None).unwrap().unwrap();

        assert_eq!(config.policy.header_max_length, Some(50));
    }

    #[test]
    fn errors_for_a_missing_explicit_config() {
        let dir = TempDir::new();
        let missing = dir.path.join("nope.json");

        assert!(load(&dir.path, Some(&missing)).is_err());
    }

    #[test]
    fn reads_javascript_config_when_node_is_available() {
        let dir = TempDir::new();
        let path = dir.write(
            "commitlint.config.cjs",
            "module.exports = { rules: { 'header-max-length': [2, 'always', 45], 'type-case': [2, 'always', 'upper-case'] } };\n",
        );

        let result = load(&dir.path, Some(&path));

        let Ok(Some(config)) = result else {
            // Node is optional; skip when it is not installed.
            return;
        };

        assert_eq!(config.policy.header_max_length, Some(45));
        assert_eq!(config.policy.type_case, Some(Case::Upper));
    }

    #[test]
    fn reads_typescript_config_when_node_is_available() {
        let dir = TempDir::new();
        let path = dir.write(
            "commitlint.config.ts",
            "export default { rules: { 'type-enum': [2, 'always', ['feat', 'fix']] } };\n",
        );

        let result = load(&dir.path, Some(&path));

        let Ok(Some(config)) = result else {
            return;
        };

        assert_eq!(
            config.policy.types,
            Some(vec!["feat".to_string(), "fix".to_string()])
        );
    }

    #[test]
    fn accepts_rules_at_the_root_of_a_config() {
        let dir = TempDir::new();
        let path = dir.write(
            ".commitlintrc.json",
            r#"{ "type-enum": [2, "always", ["wip"]] }"#,
        );

        let config = load(&dir.path, Some(&path)).unwrap().unwrap();

        assert_eq!(config.policy.types, Some(vec!["wip".to_string()]));
    }

    #[test]
    fn all_documented_config_names_are_searchable() {
        let expected = [
            "package.json",
            "package.yaml",
            ".commitlintrc",
            ".commitlintrc.json",
            ".commitlintrc.yaml",
            ".commitlintrc.yml",
            ".commitlintrc.js",
            ".commitlintrc.cjs",
            ".commitlintrc.mjs",
            ".commitlintrc.ts",
            ".commitlintrc.cts",
            ".commitlintrc.mts",
            "commitlint.config.js",
            "commitlint.config.cjs",
            "commitlint.config.mjs",
            "commitlint.config.ts",
            "commitlint.config.cts",
            "commitlint.config.mts",
        ];

        assert_eq!(SEARCH_PLACES, &expected);
    }
}
