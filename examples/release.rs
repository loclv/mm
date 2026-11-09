//! Prepare a patch release: `cargo run --example release`.
use std::fs;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match release() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed ({status})", args.join(" ")))
    }
}

fn bump_manifest(manifest: &str) -> Result<(String, String), String> {
    let mut in_package = false;
    let mut result = String::new();
    let mut version = None;
    for line in manifest.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
        }
        if in_package && trimmed.starts_with("version") {
            let (key, value) = trimmed.split_once('=').ok_or("invalid package version")?;
            if key.trim() == "version" {
                if version.is_some() {
                    return Err("duplicate package version".into());
                }
                let old = value
                    .trim()
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .ok_or("expected a quoted package version")?;
                let parts: Vec<_> = old.split('.').collect();
                if parts.len() != 3
                    || parts
                        .iter()
                        .any(|part| part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()))
                {
                    return Err("expected a numeric major.minor.patch version".into());
                }
                let patch = parts[2]
                    .parse::<u64>()
                    .map_err(|_| "invalid patch version")?
                    .checked_add(1)
                    .ok_or("patch version overflow")?;
                let new = format!("{}.{}.{patch}", parts[0], parts[1]);
                result.push_str(&format!(
                    "version = \"{new}\"{}",
                    if line.ends_with('\n') { "\n" } else { "" }
                ));
                version = Some(new);
                continue;
            }
        }
        result.push_str(line);
    }
    Ok((result, version.ok_or("package version not found")?))
}

fn release() -> Result<(), String> {
    if std::env::args_os().len() != 1 {
        return Err("usage: cargo run --example release".into());
    }
    let root = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("could not find repository: {e}"))?;
    if !root.status.success() {
        return Err("not in a Git repository".into());
    }
    let root = String::from_utf8(root.stdout).map_err(|_| "repository path is not UTF-8")?;
    std::env::set_current_dir(root.trim())
        .map_err(|e| format!("could not enter repository: {e}"))?;
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .map_err(|e| format!("could not check Git status: {e}"))?;
    if !status.status.success() || !status.stdout.is_empty() {
        return Err("working tree must be clean before preparing a release".into());
    }
    let manifest =
        fs::read_to_string("Cargo.toml").map_err(|e| format!("could not read Cargo.toml: {e}"))?;
    let (updated, version) = bump_manifest(&manifest)?;
    let tag = format!("release-v{version}");
    let exists = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/tags/{tag}"),
        ])
        .status()
        .map_err(|e| format!("could not check tag: {e}"))?;
    if exists.success() {
        return Err(format!("tag {tag} already exists"));
    }
    eprintln!("Running release checks before preparing {tag}...");
    run("./test.sh", &[])?;
    fs::write("Cargo.toml", updated).map_err(|e| format!("could not update Cargo.toml: {e}"))?;
    run("cargo", &["check"])?;
    run("git", &["add", "Cargo.toml", "Cargo.lock"])?;
    run(
        "git",
        &["commit", "-m", &format!("chore: release v{version}")],
    )?;
    run(
        "git",
        &["tag", "-a", &tag, "-m", &format!("Release v{version}")],
    )?;
    println!("Prepared {tag}. Review and push the commit and tag to publish.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::bump_manifest;
    #[test]
    fn bumps_only_package_version() {
        let (manifest, version) = bump_manifest(
            "[package]\nname = \"mm\"\nversion = \"0.2.9\"\n[dependencies]\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        assert_eq!(version, "0.2.10");
        assert!(manifest.contains("version = \"0.2.10\""));
        assert!(manifest.contains("version = \"1.0.0\""));
    }
    #[test]
    fn rejects_non_numeric_versions() {
        assert!(bump_manifest("[package]\nversion = \"1.0.0-rc1\"\n").is_err());
    }
}
