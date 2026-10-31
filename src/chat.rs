//! Copy a commit message from an OpenCode, Antigravity, Claude Code, Codex, Devin, or Freebuff chat session.
//!
//! OpenCode owns its session database, but it exposes a stable JSON export
//! through `opencode session export <id>`. The export contains the session
//! title and the visible transcript. `mm --chat` uses the title as the commit
//! subject and copies the last assistant message as the body, matching
//! OpenCode's `/copy` command. A completed compaction summary is copied when it
//! is the last message. Tool calls, reasoning and system messages are skipped.
//! Absolute user file paths in `file://` URIs (like `file:///Users/user/...`)
//! are replaced with `~/...` to prevent leaking local user names.

use std::fs::{self, OpenOptions};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::policy::truncate;

/// Environment variable OpenCode sets for the current session.
pub const SESSION_ENV: &str = "OPENCODE_SESSION_ID";

/// Environment variable for explicit Antigravity conversation ID.
pub const ANTIGRAVITY_SESSION_ENV: &str = "ANTIGRAVITY_CONVERSATION_ID";

/// Environment variable for Antigravity brain directory override (useful for tests).
pub const ANTIGRAVITY_BRAIN_ENV: &str = "ANTIGRAVITY_BRAIN_DIR";

/// Environment variable for explicit Claude Code session ID.
pub const CLAUDE_SESSION_ENV: &str = "CLAUDE_SESSION_ID";

/// Environment variable for Claude configuration directory override (useful for tests).
pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

/// Environment variable for explicit Codex session ID.
pub const CODEX_SESSION_ENV: &str = "CODEX_SESSION_ID";

/// Environment variable for Codex home directory override (useful for tests).
pub const CODEX_HOME_ENV: &str = "CODEX_HOME";

/// Environment variable for explicit Devin session ID.
pub const DEVIN_SESSION_ENV: &str = "DEVIN_SESSION_ID";

/// Environment variable for Devin home directory override (useful for tests).
pub const DEVIN_HOME_ENV: &str = "DEVIN_HOME";

/// Environment variable for explicit Freebuff session ID.
pub const FREEBUFF_SESSION_ENV: &str = "FREEBUFF_SESSION_ID";

/// Environment variable for explicit Codebuff session ID alias.
pub const CODEBUFF_SESSION_ENV: &str = "CODEBUFF_SESSION_ID";

/// Environment variable for Freebuff config directory override (useful for tests).
pub const FREEBUFF_CONFIG_DIR_ENV: &str = "FREEBUFF_CONFIG_DIR";

/// Environment variable for Codebuff config directory override (useful for tests).
pub const CODEBUFF_CONFIG_DIR_ENV: &str = "CODEBUFF_CONFIG_DIR";

/// Environment variable for Freebuff home directory override (useful for tests).
pub const FREEBUFF_HOME_ENV: &str = "FREEBUFF_HOME";

/// Environment variable for Codebuff home directory override (useful for tests).
pub const CODEBUFF_HOME_ENV: &str = "CODEBUFF_HOME";

/// Longest fallback subject, in characters.
const FALLBACK_SUBJECT_MAX: usize = 72;

/// Where the subject came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectSource {
    /// The session's stored title.
    Title,
    /// The first line of the latest user prompt, used when there is no title.
    LatestPrompt,
    /// The first line of the copied assistant message, used when there is no
    /// title or user prompt.
    LastMessage,
}

/// A chat source that `mm` can copy a commit message from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    OpenCode,
    Antigravity,
    Claude,
    Codex,
    Devin,
    Freebuff,
}

impl Agent {
    /// Agents with readable transcripts on disk, checked when resolving a
    /// session ID. OpenCode is the final fallback because it exports on demand.
    const SESSION_ORDER: [Agent; 5] = [
        Agent::Freebuff,
        Agent::Devin,
        Agent::Claude,
        Agent::Antigravity,
        Agent::Codex,
    ];

    /// Agents checked for an environment-selected session. OpenCode comes first
    /// to match its own `OPENCODE_SESSION_ID` convention.
    const ENV_ORDER: [Agent; 6] = [
        Agent::OpenCode,
        Agent::Freebuff,
        Agent::Devin,
        Agent::Claude,
        Agent::Antigravity,
        Agent::Codex,
    ];

    /// Agents that keep transcripts on disk, in auto-detection order.
    const LOCAL: [Agent; 5] = [
        Agent::Antigravity,
        Agent::Claude,
        Agent::Codex,
        Agent::Devin,
        Agent::Freebuff,
    ];

    /// Human-readable name used in diagnostics and errors.
    pub fn label(self) -> &'static str {
        match self {
            Agent::OpenCode => "OpenCode",
            Agent::Antigravity => "Antigravity",
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Devin => "Devin",
            Agent::Freebuff => "Freebuff",
        }
    }

    /// Environment variables that select a session for this agent.
    fn session_envs(self) -> &'static [&'static str] {
        match self {
            Agent::OpenCode => &[SESSION_ENV],
            Agent::Antigravity => &[ANTIGRAVITY_SESSION_ENV, "GEMINI_CONVERSATION_ID"],
            Agent::Claude => &[CLAUDE_SESSION_ENV],
            Agent::Codex => &[CODEX_SESSION_ENV],
            Agent::Devin => &[DEVIN_SESSION_ENV],
            Agent::Freebuff => &[FREEBUFF_SESSION_ENV, CODEBUFF_SESSION_ENV],
        }
    }

    /// Resolve an explicit session ID or transcript path.
    fn resolve(self, session: &str, repo_root: Option<&Path>) -> Option<(String, PathBuf)> {
        match self {
            Agent::OpenCode => None,
            Agent::Antigravity => resolve_antigravity_session(session),
            Agent::Claude => resolve_claude_session(session),
            Agent::Codex => resolve_codex_session(session),
            Agent::Devin => resolve_devin_session(session),
            Agent::Freebuff => resolve_freebuff_session(session, repo_root),
        }
    }

    /// Find the most recently modified transcript on disk.
    fn find_latest(self, repo_root: Option<&Path>) -> Option<(String, PathBuf)> {
        match self {
            Agent::OpenCode => None,
            Agent::Antigravity => find_latest_antigravity_log(repo_root),
            Agent::Claude => find_latest_claude_log(repo_root),
            Agent::Codex => find_latest_codex_log(repo_root),
            Agent::Devin => find_latest_devin_log(repo_root),
            Agent::Freebuff => find_latest_freebuff_log(repo_root),
        }
    }

    /// Parse a transcript found on disk.
    fn parse(
        self,
        path: &Path,
        session_id: &str,
        repo_root: Option<&Path>,
    ) -> Result<ChatMessage, String> {
        match self {
            Agent::Antigravity => load_antigravity(path, session_id, repo_root),
            Agent::Claude => load_claude(path, session_id, repo_root),
            Agent::Codex => load_codex(path, session_id, repo_root),
            Agent::Devin => load_devin(path, session_id, repo_root),
            Agent::Freebuff => load_freebuff(path, session_id, repo_root),
            Agent::OpenCode => {
                Err("OpenCode sessions are exported, not read from disk".to_string())
            }
        }
    }

    /// Return the first session ID from the agent's environment variables, if
    /// any is set (an explicitly empty value is kept so callers can reject it).
    fn env_session(self) -> Option<String> {
        self.session_envs().iter().find_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| value.trim().to_string())
        })
    }
}

/// A commit message copied from an OpenCode or Antigravity session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Session the message came from.
    pub session_id: String,
    /// Subject line.
    pub subject: String,
    /// The last assistant text or completed compaction summary, like `/copy`.
    pub body: String,
    /// Whether [`ChatMessage::subject`] is the session title or a fallback.
    pub subject_source: SubjectSource,
}

impl ChatMessage {
    /// Render the final message, optionally replacing the subject.
    pub fn message(&self, subject_override: Option<&str>) -> String {
        self.message_with_root(subject_override, None)
    }

    /// Format message and sanitize current repository paths using `repo_root`.
    pub fn message_with_root(
        &self,
        subject_override: Option<&str>,
        repo_root: Option<&Path>,
    ) -> String {
        let subject = subject_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.subject);

        let body = sanitize_chat_body(&self.body, repo_root);

        let msg = if body.is_empty() {
            subject.to_string()
        } else {
            format!("{subject}\n\n{body}")
        };

        sanitize_current_path(&msg, repo_root)
    }
}

/// Check if a character acts as a path or markdown delimiter.
fn is_path_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '`'
                | ')'
                | ']'
                | '}'
                | '>'
                | '<'
                | '\\'
                | ';'
                | ','
                | ':'
                | '?'
                | '!'
                | '|'
                | '*'
        )
}

/// Strip a prefix case-insensitively.
fn strip_prefix_case_insensitive<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Try matching a user home directory path in the remainder after `file://`.
/// Returns `(consumed_bytes, replacement)` on success.
fn try_match_file_url(after_scheme: &str, home: Option<&str>) -> Option<(usize, &'static str)> {
    let (rest, host_len) = if let Some(stripped) = after_scheme.strip_prefix("localhost") {
        (stripped, "localhost".len())
    } else {
        (after_scheme, 0)
    };

    // Check custom $HOME if available
    if let Some(home_path) = home
        && let Some(after_home) = rest.strip_prefix(home_path)
    {
        if after_home.starts_with('/') {
            return Some((host_len + home_path.len() + 1, "~/"));
        } else if after_home.is_empty() || is_path_delimiter(after_home.chars().next().unwrap()) {
            return Some((host_len + home_path.len(), "~"));
        }
    }

    // Check Windows drive letter: e.g. `/C:/` or `C:/`
    let (rest_after_drive, drive_len) = if rest.len() >= 4
        && rest.as_bytes()[0] == b'/'
        && rest.as_bytes()[1].is_ascii_alphabetic()
        && rest.as_bytes()[2] == b':'
        && rest.as_bytes()[3] == b'/'
    {
        (&rest[3..], 3)
    } else if rest.len() >= 3
        && rest.as_bytes()[0].is_ascii_alphabetic()
        && rest.as_bytes()[1] == b':'
        && rest.as_bytes()[2] == b'/'
    {
        (&rest[2..], 2)
    } else {
        (rest, 0)
    };

    // Check standard user directory prefixes
    let known_prefixes = ["/Users/", "Users/", "/home/", "home/"];
    let mut matched = None;

    for prefix in &known_prefixes {
        if let Some(after_prefix) = strip_prefix_case_insensitive(rest_after_drive, prefix) {
            matched = Some((after_prefix, prefix.len()));
            break;
        }
    }

    let (after_prefix, matched_prefix_len) = matched?;
    let prefix_total_len = host_len + drive_len + matched_prefix_len;

    // Scan for username
    let mut user_len = None;
    let mut found_slash = false;

    for (i, c) in after_prefix.char_indices() {
        if c == '/' {
            user_len = Some(i);
            found_slash = true;
            break;
        }
        if is_path_delimiter(c) {
            user_len = Some(i);
            break;
        }
    }

    let user_len = user_len.unwrap_or(after_prefix.len());

    if user_len == 0 {
        return None;
    }

    if found_slash {
        Some((prefix_total_len + user_len + 1, "~/"))
    } else {
        Some((prefix_total_len + user_len, "~"))
    }
}

/// Replace user home file URIs (`file:///Users/<user>/...`, `file:///home/<user>/...`, etc.)
/// with `~/...` so that local user names are not leaked into commit messages.
pub fn sanitize_file_urls(text: &str) -> String {
    if !text.contains("file://") {
        return text.to_string();
    }

    let home = std::env::var("HOME").ok().and_then(|h| {
        let trimmed = h.trim().trim_end_matches('/');
        if trimmed.len() > 1 && trimmed != "/" {
            Some(trimmed.to_string())
        } else {
            None
        }
    });

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(rel_pos) = text[cursor..].find("file://") {
        let file_pos = cursor + rel_pos;
        result.push_str(&text[cursor..file_pos]);

        let after_scheme = &text[file_pos + "file://".len()..];

        if let Some((consumed_len, replacement)) = try_match_file_url(after_scheme, home.as_deref())
        {
            result.push_str(replacement);
            cursor = file_pos + "file://".len() + consumed_len;
        } else {
            result.push_str("file://");
            cursor = file_pos + "file://".len();
        }
    }

    result.push_str(&text[cursor..]);
    result
}

/// Replace occurrences of `target/` with `replacement`, and occurrences of `target`
/// followed by a path delimiter or end-of-string with `replacement`.
fn replace_path_target(mut text: String, target: &str, replacement: &str) -> String {
    let with_slash = format!("{target}/");
    text = text.replace(&with_slash, replacement);

    let target_len = target.len();
    let mut start = 0;
    while let Some(pos) = text[start..].find(target) {
        let abs_pos = start + pos;
        let after = abs_pos + target_len;
        let is_boundary = if after == text.len() {
            true
        } else {
            let next_char = text[after..].chars().next().unwrap();
            if is_path_delimiter(next_char) {
                true
            } else if next_char == '.' {
                let after_dot = after + next_char.len_utf8();
                after_dot == text.len()
                    || !text[after_dot..].chars().next().unwrap().is_alphanumeric()
            } else {
                false
            }
        };

        if is_boundary {
            text.replace_range(abs_pos..after, replacement);
            start = abs_pos + replacement.len();
        } else {
            start = after;
        }
    }
    text
}

/// Replace occurrences of the current path (e.g. `~/w/auto-commit/rust/`,
/// `file:///Users/user/repo/`, or `/Users/user/repo/`) with `./` in commit messages.
pub fn sanitize_current_path_with_home(
    text: &str,
    current_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> String {
    let dir = match current_dir {
        Some(d) => d.to_path_buf(),
        None => match std::env::current_dir() {
            Ok(d) => d,
            Err(_) => return text.to_string(),
        },
    };

    let mut dirs = vec![dir.clone()];
    if let Ok(canonical) = fs::canonicalize(&dir)
        && !dirs.contains(&canonical)
    {
        dirs.push(canonical);
    }
    let dir_str = dir.to_string_lossy();
    if let Some(stripped) = dir_str.strip_prefix("/private/") {
        let alt = PathBuf::from(format!("/{stripped}"));
        if !dirs.contains(&alt) {
            dirs.push(alt);
        }
    }

    // If dir is inside a git repository, also add the repository root to dirs
    let mut check_parent = dir.as_path();
    while let Some(parent) = check_parent.parent() {
        if parent.join(".git").exists() {
            if !dirs.contains(&parent.to_path_buf()) {
                dirs.push(parent.to_path_buf());
                if let Ok(canonical) = fs::canonicalize(parent)
                    && !dirs.contains(&canonical)
                {
                    dirs.push(canonical);
                }
                let p_str = parent.to_string_lossy();
                if let Some(stripped) = p_str.strip_prefix("/private/") {
                    let alt = PathBuf::from(format!("/{stripped}"));
                    if !dirs.contains(&alt) {
                        dirs.push(alt);
                    }
                }
            }
            break;
        }
        check_parent = parent;
    }

    let mut home_paths = Vec::new();
    if let Some(h) = home_dir {
        home_paths.push(h.to_path_buf());
        if let Ok(canonical) = fs::canonicalize(h)
            && !home_paths.contains(&canonical)
        {
            home_paths.push(canonical);
        }
        let h_str = h.to_string_lossy();
        if let Some(stripped) = h_str.strip_prefix("/private/") {
            let alt = PathBuf::from(format!("/{stripped}"));
            if !home_paths.contains(&alt) {
                home_paths.push(alt);
            }
        }
    }

    let mut result = text.to_string();

    for d in &dirs {
        let abs_str = d.to_string_lossy().replace('\\', "/");
        let abs_trimmed = abs_str.trim_end_matches('/');
        if abs_trimmed.is_empty() || abs_trimmed == "/" {
            continue;
        }

        // 1. file://localhost<abs>
        let file_localhost = format!("file://localhost{abs_trimmed}");
        result = replace_path_target(result, &file_localhost, "./");

        // 2. file://<abs>
        let file_direct = format!("file://{abs_trimmed}");
        result = replace_path_target(result, &file_direct, "./");

        // 3. ~/<rel> if inside HOME
        for home_path in &home_paths {
            if let Ok(rel) = d.strip_prefix(home_path) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let rel_trimmed = rel_str.trim_matches('/');
                if !rel_trimmed.is_empty() {
                    let home_rel = format!("~/{rel_trimmed}");
                    result = replace_path_target(result, &home_rel, "./");
                    let file_home = format!("file://{home_rel}");
                    result = replace_path_target(result, &file_home, "./");
                    let file_home_slash = format!("file:///{home_rel}");
                    result = replace_path_target(result, &file_home_slash, "./");
                }
            }
        }

        // 4. raw <abs>
        result = replace_path_target(result, abs_trimmed, "./");
    }

    result = result.replace("file:///./", "./");
    result = result.replace("file://./", "./");

    result
}

/// Replace current directory paths (e.g. `~/w/auto-commit/rust/`, `file:///Users/user/repo/`,
/// or `/Users/user/repo/`) with `./` in commit messages using `$HOME`.
pub fn sanitize_current_path(text: &str, current_dir: Option<&Path>) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    sanitize_current_path_with_home(text, current_dir, home.as_deref())
}

/// Strip ANSI escape sequences and carriage return artifacts from terminal logs.
pub fn strip_ansi_escapes(text: &str) -> String {
    if !text.contains('\x1b') && !text.contains('\r') {
        return text.to_string();
    }

    let bytes = text.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\x1b' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    // CSI sequence: ESC [ [0x30-0x3F]* [0x20-0x2F]* [0x40-0x7E]
                    i += 1;
                    while i < bytes.len() && (0x30..=0x3f).contains(&bytes[i]) {
                        i += 1;
                    }
                    while i < bytes.len() && (0x20..=0x2f).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() && (0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                }
                b']' => {
                    // OSC sequence: ESC ] ... (BEL \x07 or ST ESC \)
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        } else if bytes[i] == b'\x1b'
                            && i + 1 < bytes.len()
                            && bytes[i + 1] == b'\\'
                        {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                b'(' | b')' | b'*' | b'+' => {
                    // Character set designation: 2 bytes
                    i += 2;
                }
                b'P' | b'X' | b'^' | b'_' => {
                    // DCS, SOS, PM, APC - terminated by ST or BEL
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        } else if bytes[i] == b'\x1b'
                            && i + 1 < bytes.len()
                            && bytes[i + 1] == b'\\'
                        {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                0x40..=0x5f => {
                    // 2-byte escape sequence
                    i += 1;
                }
                _ => {
                    // Skip unrecognized escape byte
                    i += 1;
                }
            }
        } else if bytes[i] == b'\r' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                result.push(b'\n');
                i += 2;
            } else {
                // Standalone \r
                i += 1;
            }
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }

    String::from_utf8(result).unwrap_or_else(|_| text.to_string())
}

/// Strip paired XML/HTML tag blocks including their interior content.
fn strip_xml_tag_block(text: &str, tag_name: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remainder = text;
    let open_prefix = format!("<{tag_name}");
    let close_tag = format!("</{tag_name}>");

    while let Some(start_idx) = remainder.to_ascii_lowercase().find(&open_prefix) {
        let after_start = &remainder[start_idx + open_prefix.len()..];
        let next_char = after_start.chars().next();
        let valid_start = matches!(next_char, Some('>' | ' ' | '\t' | '\r' | '\n' | '/'));

        if !valid_start {
            result.push_str(&remainder[..start_idx + open_prefix.len()]);
            remainder = after_start;
            continue;
        }

        result.push_str(&remainder[..start_idx]);

        if let Some(tag_end) = after_start.find('>') {
            let inside_tag = after_start[..tag_end].trim_end();
            if inside_tag.ends_with('/') {
                // Self-closing: <tag ... />
                remainder = &after_start[tag_end + 1..];
                continue;
            }

            let after_open_tag = &after_start[tag_end + 1..];
            if let Some(close_idx) = after_open_tag.to_ascii_lowercase().find(&close_tag) {
                remainder = &after_open_tag[close_idx + close_tag.len()..];
            } else {
                // Unclosed opening tag, drop until end or next line
                remainder = &after_start[tag_end + 1..];
            }
        } else {
            remainder = after_start;
        }
    }

    result.push_str(remainder);
    result
}

/// Strip `<details><summary>...Thinking/Tool...</summary>...</details>` blocks.
fn strip_details_tool_blocks(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remainder = text;

    while let Some(start_idx) = remainder.to_ascii_lowercase().find("<details") {
        let after_start = &remainder[start_idx..];
        let lower_after = after_start.to_ascii_lowercase();

        if let Some(summary_idx) = lower_after.find("<summary") {
            let after_summary = &lower_after[summary_idx..];
            if let Some(summary_end) = after_summary.find("</summary>") {
                let summary_content = &after_summary[..summary_end];
                let is_tool_summary = summary_content.contains("thinking")
                    || summary_content.contains("thought")
                    || summary_content.contains("tool")
                    || summary_content.contains("command")
                    || summary_content.contains("ran ");

                if is_tool_summary {
                    result.push_str(&remainder[..start_idx]);
                    if let Some(details_end) = lower_after.find("</details>") {
                        remainder = &after_start[details_end + "</details>".len()..];
                        continue;
                    } else {
                        // Unclosed <details>, drop until end
                        remainder = "";
                        break;
                    }
                }
            }
        }

        result.push_str(&remainder[..start_idx + "<details".len()]);
        remainder = &remainder[start_idx + "<details".len()..];
    }

    result.push_str(remainder);
    result
}

/// Check if code block fence info string indicates a tool invocation or result.
fn is_tool_fence_info(info: &str) -> bool {
    let lower = info.to_ascii_lowercase();
    lower.starts_with("tool_call")
        || lower.starts_with("tool_use")
        || lower.starts_with("tool_result")
        || lower.starts_with("tool_output")
        || lower.starts_with("function_call")
        || lower.contains(":tool_call")
        || lower.contains(":tool_use")
}

/// Check if a trimmed line is a tool/thought marker or raw tool JSON.
fn is_tool_marker_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();

    let markers = [
        "action:",
        "action input:",
        "observation:",
        "[action:",
        "[action ",
        "[observation:",
        "[tool call:",
        "[tool use:",
        "[tool result:",
        "[tool output:",
        "[tool:",
        "[running command:",
        "[command output:",
        "[command stderr:",
        "[command stdout:",
        "command stdout:",
        "command stderr:",
        "tool call:",
        "tool result:",
        "tool output:",
        "executing tool:",
        "calling tool:",
        "ran command:",
        "thinking process:",
        "thinking:",
    ];

    for marker in &markers {
        if lower.starts_with(marker) {
            return true;
        }
    }

    let lone_tags = [
        "</tool_call>",
        "<tool_call>",
        "</tool_use>",
        "<tool_use>",
        "</tool_result>",
        "<tool_result>",
        "</tool_output>",
        "<tool_output>",
        "</thought>",
        "<thought>",
        "</thinking>",
        "<thinking>",
        "</think>",
        "<think>",
        "</function_call>",
        "<function_call>",
    ];

    for tag in &lone_tags {
        if lower == *tag {
            return true;
        }
    }

    if lower.starts_with('{')
        && lower.ends_with('}')
        && ((lower.contains("\"name\":")
            && (lower.contains("\"arguments\":") || lower.contains("\"parameters\":")))
            || lower.contains("\"tool\":")
            || lower.contains("\"tool_name\":")
            || lower.contains("\"tool_input\":"))
    {
        return true;
    }

    false
}

/// Strip tool call residue (XML tags, tool code blocks, and ReAct/CLI markers).
pub fn strip_tool_residue(text: &str) -> String {
    let mut current = text.to_string();

    let xml_tags = [
        "tool_call",
        "tool_use",
        "tool_result",
        "tool_output",
        "function_call",
        "function_calls",
        "antml:function_call",
        "thought",
        "thinking",
        "think",
    ];

    for tag in xml_tags {
        current = strip_xml_tag_block(&current, tag);
    }

    current = strip_xml_tag_block(&current, "invoke");
    current = strip_details_tool_blocks(&current);

    let mut lines = Vec::new();
    let mut in_tool_fence = false;

    for line in current.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            let after_fence = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
            if !in_tool_fence {
                if is_tool_fence_info(&after_fence) {
                    in_tool_fence = true;
                    continue;
                }
            } else {
                in_tool_fence = false;
                continue;
            }
        }

        if in_tool_fence {
            continue;
        }

        if is_tool_marker_line(trimmed) {
            continue;
        }

        lines.push(line);
    }

    collapse_blank_lines(&lines.join("\n"))
}

/// Check if a line is a structural markdown element (heading, list item, or code fence)
/// that should never be considered conversational filler.
fn is_structural_line(trimmed: &str) -> bool {
    trimmed.starts_with('#')
        || trimmed.starts_with("```")
        || trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || (trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) && trimmed.contains(". "))
}

/// Unwrap enclosing markdown bold/italic formatting.
fn unwrap_markdown_emphasis(s: &str) -> &str {
    let trimmed = s.trim();
    if (trimmed.starts_with("**") && trimmed.ends_with("**") && trimmed.len() >= 4)
        || (trimmed.starts_with("__") && trimmed.ends_with("__") && trimmed.len() >= 4)
    {
        &trimmed[2..trimmed.len() - 2]
    } else if (trimmed.starts_with('*') && trimmed.ends_with('*') && trimmed.len() >= 2)
        || (trimmed.starts_with('_') && trimmed.ends_with('_') && trimmed.len() >= 2)
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

/// Check if a normalized line is an entire conversational filler phrase.
fn is_full_filler_phrase(lower: &str) -> bool {
    let trimmed = lower.trim().trim_end_matches(['.', '!', '?', ':']);

    let affirmations = [
        "sure",
        "sure thing",
        "certainly",
        "of course",
        "definitely",
        "absolutely",
        "okay",
        "ok",
        "alright",
        "all right",
        "great",
        "awesome",
        "no problem",
        "happy to help",
    ];

    for aff in &affirmations {
        if trimmed == *aff {
            return true;
        }
    }

    let promise_starts = [
        "i will help you with",
        "i'll help you with",
        "i can help you with",
        "i can help with",
        "i'd be happy to help",
        "i would be happy to help",
        "i am happy to help",
        "i'm happy to help",
        "i will assist you with",
        "i can assist you with",
        "i can assist with",
        "i'd be glad to help",
        "i'll be glad to help",
        "let me help you with",
        "let's get started",
    ];

    for start in &promise_starts {
        if trimmed.starts_with(start) {
            return true;
        }
    }

    let leadin_exact = ["summary", "summary of changes", "summary of the changes"];

    for exact in &leadin_exact {
        if trimmed == *exact {
            return true;
        }
    }

    let leadin_starts = [
        "here is a summary",
        "here's a summary",
        "here is the summary",
        "here's the summary",
        "here is a breakdown",
        "here's a breakdown",
        "here are the changes",
        "here are the updates",
        "here's what changed",
        "here is what changed",
        "here's what i did",
        "here is what i did",
        "here is what was done",
        "here's what was done",
        "here is what i have done",
        "here's what i've done",
        "below is a summary",
        "below is the summary",
        "below are the changes",
        "below are the updates",
        "the following changes were made",
        "the following changes have been made",
        "the following is a summary",
        "the following is a list of changes",
    ];

    for start in &leadin_starts {
        if trimmed.starts_with(start) {
            return true;
        }
    }

    false
}

/// Strip leading conversational filler prefix from a line if present.
fn strip_leading_filler_prefix(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase();

    let affirmative_prefixes = [
        "sure thing!",
        "sure thing,",
        "sure thing.",
        "sure!",
        "sure,",
        "sure.",
        "certainly!",
        "certainly,",
        "certainly.",
        "of course!",
        "of course,",
        "of course.",
        "definitely!",
        "definitely,",
        "okay,",
        "okay!",
        "ok,",
        "ok!",
        "alright,",
        "alright!",
        "all right,",
        "all right!",
        "great!",
        "awesome!",
        "no problem!",
    ];

    for prefix in &affirmative_prefixes {
        if lower.starts_with(prefix) {
            return Some(&s[prefix.len()..]);
        }
    }

    let promise_prefixes = [
        "i will help you with that.",
        "i will help you with this.",
        "i'll help you with that.",
        "i'll help you with this.",
        "i can help you with that.",
        "i can help with that.",
        "i'd be happy to help with that.",
        "i would be happy to help with that.",
    ];

    for prefix in &promise_prefixes {
        if lower.starts_with(prefix) {
            return Some(&s[prefix.len()..]);
        }
    }

    let leadin_prefixes = [
        "here is a summary of the changes:",
        "here's a summary of the changes:",
        "here is a summary of changes:",
        "here's a summary of changes:",
        "here is a summary of the updates:",
        "here's a summary of the updates:",
        "here is a summary of what was done:",
        "here's a summary of what was done:",
        "here is a summary of what changed:",
        "here's a summary of what changed:",
        "here is a summary of what i did:",
        "here's what i did:",
        "here is what i did:",
        "here is what was done:",
        "here's what was done:",
        "here's what changed:",
        "here is what changed:",
        "here are the changes:",
        "here are the updates:",
        "here is the summary:",
        "here's the summary:",
        "here is a summary:",
        "here's a summary:",
        "here is a breakdown of the changes:",
        "here's a breakdown of the changes:",
        "here is a breakdown:",
        "here's a breakdown:",
        "below is a summary of the changes:",
        "below is a summary of changes:",
        "below is a summary:",
        "below is the summary:",
        "below are the changes:",
        "below are the updates:",
        "the following changes were made:",
        "the following changes have been made:",
        "the following is a summary of the changes:",
        "the following is a summary of changes:",
        "the following is a summary:",
        "summary of changes:",
        "summary of the changes:",
        "summary:",
    ];

    for prefix in &leadin_prefixes {
        if lower.starts_with(prefix) {
            return Some(&s[prefix.len()..]);
        }
    }

    None
}

/// Clean conversational filler from a single line, stripping prefixes iteratively.
fn clean_leading_filler_from_line(line: &str) -> String {
    let mut current = line.trim();
    let mut stripped_any = false;

    loop {
        let unwrapped = unwrap_markdown_emphasis(current);
        let lower = unwrapped.to_ascii_lowercase();

        if is_full_filler_phrase(&lower) {
            return String::new();
        }

        if let Some(rest) = strip_leading_filler_prefix(current) {
            let next_trimmed = rest.trim_start();
            if next_trimmed == current {
                break;
            }
            current = next_trimmed;
            stripped_any = true;
        } else {
            break;
        }
    }

    if current.is_empty() {
        return String::new();
    }

    if stripped_any {
        let mut chars = current.chars();
        if let Some(first) = chars.next()
            && first.is_ascii_lowercase()
        {
            let upper = first.to_ascii_uppercase();
            let mut capitalized = String::with_capacity(current.len());
            capitalized.push(upper);
            capitalized.push_str(chars.as_str());
            return capitalized;
        }
    }

    current.to_string()
}

/// Check if a line is trailing conversational filler (polite sign-off, offer for help).
fn is_trailing_filler_line(line: &str) -> bool {
    let trimmed = line.trim().trim_end_matches(['.', '!', '?', '\n', '\r']);
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();

    let trailing_patterns = [
        "let me know if you need anything else",
        "let me know if you have any questions",
        "let me know if you need any further",
        "let me know if you'd like me to",
        "let me know if you would like me to",
        "let me know if you want me to",
        "let me know if there's anything else",
        "let me know if there is anything else",
        "let me know what you think",
        "please let me know if you have any questions",
        "please let me know if you need anything else",
        "please let me know if you need any further",
        "please let me know if you'd like",
        "feel free to ask if you have any questions",
        "feel free to reach out",
        "feel free to let me know",
        "hope this helps",
        "hope that helps",
        "does this look good to you",
        "does this look good",
        "if you have any questions, feel free",
        "if you have any questions, let me know",
        "if you need any further assistance",
        "if you need any more changes, please let me know",
    ];

    for pattern in &trailing_patterns {
        if lower.contains(pattern) {
            return true;
        }
    }

    false
}

/// Collapse runs of 3 or more newlines to at most 2 newlines (\n\n).
fn collapse_blank_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut consecutive_newlines = 0;

    for c in text.chars() {
        if c == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                result.push('\n');
            }
        } else {
            consecutive_newlines = 0;
            result.push(c);
        }
    }

    result.trim().to_string()
}

/// Strip conversational filler from assistant responses (leading greetings, affirmations,
/// promises, summary lead-ins, and trailing polite sign-offs).
pub fn strip_conversational_filler(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut start = 0;
    let mut modified_first_line: Option<String> = None;

    while start < lines.len() {
        let line = lines[start];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            start += 1;
            continue;
        }

        if is_structural_line(trimmed) {
            break;
        }

        let cleaned = clean_leading_filler_from_line(trimmed);
        if cleaned.is_empty() {
            start += 1;
            continue;
        } else if cleaned != trimmed {
            modified_first_line = Some(cleaned);
            break;
        } else {
            break;
        }
    }

    let mut end = lines.len();
    while end > start {
        let line = lines[end - 1];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            end -= 1;
            continue;
        }

        if is_structural_line(trimmed) {
            break;
        }

        if is_trailing_filler_line(trimmed) {
            end -= 1;
            continue;
        } else {
            break;
        }
    }

    if start >= end {
        return String::new();
    }

    let mut result_lines = Vec::new();
    for (idx, line) in lines[start..end].iter().enumerate() {
        if idx == 0
            && let Some(ref first) = modified_first_line
        {
            result_lines.push(first.as_str());
        } else {
            result_lines.push(*line);
        }
    }

    let joined = result_lines.join("\n");
    collapse_blank_lines(&joined)
}

/// Sanitize chat body by:
/// 1. Stripping ANSI escape sequences and carriage return artifacts.
/// 2. Stripping tool call and thought residue (XML tags, code fences, CLI/ReAct markers).
/// 3. Stripping conversational filler (greetings, promises, lead-ins, sign-offs).
/// 4. Sanitizing user file URLs (`file:///Users/...` -> `~/...`).
/// 5. Sanitizing repository paths (`file:///.../repo/` -> `./`).
pub fn sanitize_chat_body(text: &str, repo_root: Option<&Path>) -> String {
    let no_ansi = strip_ansi_escapes(text);
    let no_tools = strip_tool_residue(&no_ansi);
    let no_filler = strip_conversational_filler(&no_tools);
    let no_urls = sanitize_file_urls(&no_filler);
    sanitize_current_path(&no_urls, repo_root)
}

/// Path to Antigravity's brain directory storing conversation transcripts.
pub fn antigravity_brain_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(ANTIGRAVITY_BRAIN_ENV) {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            return Some(path);
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let brain = home.join(".gemini/antigravity-ide/brain");
    if brain.is_dir() { Some(brain) } else { None }
}

/// Resolve an Antigravity conversation by session ID, folder name, or file path.
pub fn resolve_antigravity_session(session: &str) -> Option<(String, PathBuf)> {
    let path = Path::new(session);
    if path.is_file() {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("antigravity")
            .to_string();
        return Some((name, path.to_path_buf()));
    }
    if path.is_dir() {
        let log = path.join(".system_generated/logs/transcript.jsonl");
        if log.is_file() {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("antigravity")
                .to_string();
            return Some((name, log));
        }
        let alt = path.join("transcript.jsonl");
        if alt.is_file() {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("antigravity")
                .to_string();
            return Some((name, alt));
        }
    }

    if let Some(brain) = antigravity_brain_dir() {
        let dir = brain.join(session);
        if dir.is_dir() {
            let log = dir.join(".system_generated/logs/transcript.jsonl");
            if log.is_file() {
                return Some((session.to_string(), log));
            }
            let alt = dir.join("transcript.jsonl");
            if alt.is_file() {
                return Some((session.to_string(), alt));
            }
        }

        if let Ok(entries) = fs::read_dir(&brain) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(session) && entry.path().is_dir() {
                    let log = entry.path().join(".system_generated/logs/transcript.jsonl");
                    if log.is_file() {
                        return Some((name, log));
                    }
                    let alt = entry.path().join("transcript.jsonl");
                    if alt.is_file() {
                        return Some((name, alt));
                    }
                }
            }
        }
    }

    None
}

/// Find the latest Antigravity conversation log.
pub fn find_latest_antigravity_log(repo_root: Option<&Path>) -> Option<(String, PathBuf)> {
    let brain = antigravity_brain_dir()?;
    let entries = fs::read_dir(&brain).ok()?;

    let mut candidates: Vec<(SystemTime, String, PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if name == "tempmediaStorage" {
            continue;
        }

        let log = path.join(".system_generated/logs/transcript.jsonl");
        let target_log = if log.is_file() {
            log
        } else {
            let alt = path.join("transcript.jsonl");
            if alt.is_file() {
                alt
            } else {
                continue;
            }
        };

        let mtime = fs::metadata(&target_log)
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);

        candidates.push((mtime, name, target_log));
    }

    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));

    if candidates.is_empty() {
        return None;
    }

    if let Some(root) = repo_root {
        let root_str = root.to_string_lossy();
        for (_, name, log_path) in candidates.iter().take(20) {
            if let Ok(mut file) = fs::File::open(log_path) {
                use std::io::Read;
                let mut buffer = [0u8; 65536];
                if let Ok(n) = file.read(&mut buffer) {
                    let sample = String::from_utf8_lossy(&buffer[..n]);
                    if sample.contains(&*root_str) {
                        return Some((name.clone(), log_path.clone()));
                    }
                }
            }
        }
    }

    let (_, name, log_path) = candidates.remove(0);
    Some((name, log_path))
}

/// Extract user request prompt from Antigravity content.
fn extract_user_request(content: &str) -> String {
    let text = if let Some(start) = content.find("<USER_REQUEST>") {
        let after = &content[start + "<USER_REQUEST>".len()..];
        if let Some(end) = after.find("</USER_REQUEST>") {
            &after[..end]
        } else {
            after
        }
    } else if let Some(meta_start) = content.find("<ADDITIONAL_METADATA>") {
        &content[..meta_start]
    } else {
        content
    };

    clean_prompt_line(text)
}

/// Clean up leading @-mentions from prompt line.
fn clean_prompt_line(text: &str) -> String {
    let trimmed = text.trim();
    if let Some((rest, idx)) = trimmed
        .strip_prefix('@')
        .and_then(|r| r.find(']').map(|i| (r, i)))
    {
        let after = rest[idx + 1..].trim();
        let stripped = after.strip_prefix(':').unwrap_or(after).trim();
        if !stripped.is_empty() {
            return stripped.to_string();
        }
    }
    trimmed.to_string()
}

/// Parse an Antigravity transcript from a reader.
pub fn parse_antigravity_reader<R: BufRead>(
    reader: R,
    conversation_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let mut latest_prompt: Option<String> = None;
    let mut last_message: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let source = value.get("source").and_then(Value::as_str).unwrap_or("");
        let msg_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        if (source == "USER_EXPLICIT" || source == "USER") && msg_type == "USER_INPUT" {
            if let Some(content) = value.get("content").and_then(Value::as_str) {
                let user_request = extract_user_request(content);
                if let Some(first) = first_line(&user_request) {
                    let cleaned = clean_prompt_line(&first);
                    if !cleaned.is_empty() {
                        latest_prompt = Some(sanitize_file_urls(&cleaned));
                    }
                }
            }
        } else if source == "MODEL" && msg_type == "PLANNER_RESPONSE" {
            let text = value
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if !text.is_empty() {
                last_message = Some(sanitize_file_urls(text));
            }
        }
    }

    let raw_body = last_message.ok_or_else(|| {
        format!(
            "Antigravity conversation `{conversation_id}` has no assistant response text to copy"
        )
    })?;

    let sanitized_body = sanitize_chat_body(&raw_body, repo_root);

    let (subject, subject_source) = match latest_prompt {
        Some(prompt) => {
            let sanitized = sanitize_current_path(&sanitize_file_urls(&prompt), repo_root);
            (
                truncate(&sanitized, FALLBACK_SUBJECT_MAX),
                SubjectSource::LatestPrompt,
            )
        }
        None => {
            let line = first_line(&sanitized_body).unwrap_or_else(|| sanitized_body.clone());
            let sanitized = sanitize_current_path(&sanitize_file_urls(&line), repo_root);
            (
                truncate(&sanitized, FALLBACK_SUBJECT_MAX),
                SubjectSource::LastMessage,
            )
        }
    };

    Ok(ChatMessage {
        session_id: conversation_id.to_string(),
        subject: sanitize_file_urls(&subject),
        body: sanitized_body,
        subject_source,
    })
}

/// Parse an Antigravity transcript from a byte slice.
pub fn parse_antigravity(transcript: &[u8], conversation_id: &str) -> Result<ChatMessage, String> {
    parse_antigravity_reader(std::io::Cursor::new(transcript), conversation_id, None)
}

/// Load and parse an Antigravity transcript from disk.
pub fn load_antigravity(
    path: &Path,
    conversation_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "could not open Antigravity transcript {}: {error}",
            path.display()
        )
    })?;
    let reader = std::io::BufReader::new(file);
    parse_antigravity_reader(reader, conversation_id, repo_root)
}

/// Path to Claude Code's configuration directory.
pub fn claude_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(CLAUDE_CONFIG_DIR_ENV) {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            return Some(path);
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let claude = home.join(".claude");
    if claude.is_dir() { Some(claude) } else { None }
}

/// Encode a filesystem path for Claude Code's project directory structure.
pub fn encode_claude_project_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let trimmed = s.trim_end_matches('/');
    let mut encoded = String::with_capacity(trimmed.len() + 1);
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() {
            encoded.push(c);
        } else {
            encoded.push('-');
        }
    }
    encoded
}

/// Resolve a Claude Code session inside a given `.claude` directory.
pub fn resolve_claude_session_in(claude_dir: &Path, session: &str) -> Option<(String, PathBuf)> {
    let path = Path::new(session);
    if path.is_file() {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("claude")
            .to_string();
        return Some((name, path.to_path_buf()));
    }
    if path.is_dir() {
        let mut candidates = Vec::new();
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    let mtime = fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH);
                    let stem = p
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("claude")
                        .to_string();
                    candidates.push((mtime, stem, p));
                }
            }
        }
        candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        if let Some((_, name, p)) = candidates.into_iter().next() {
            return Some((name, p));
        }
    }

    let projects_dir = claude_dir.join("projects");
    if projects_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&projects_dir)
    {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let direct = entry.path().join(format!("{session}.jsonl"));
                if direct.is_file() {
                    return Some((session.to_string(), direct));
                }
                let nested = entry
                    .path()
                    .join("sessions")
                    .join(format!("{session}.jsonl"));
                if nested.is_file() {
                    return Some((session.to_string(), nested));
                }
            }
        }
    }

    let direct_session = claude_dir.join("sessions").join(format!("{session}.jsonl"));
    if direct_session.is_file() {
        return Some((session.to_string(), direct_session));
    }

    None
}

/// Resolve a Claude Code session by session ID or file path.
pub fn resolve_claude_session(session: &str) -> Option<(String, PathBuf)> {
    let path = Path::new(session);
    if path.is_file() {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("claude")
            .to_string();
        return Some((name, path.to_path_buf()));
    }

    let claude_dir = claude_config_dir()?;
    resolve_claude_session_in(&claude_dir, session)
}

/// Collect .jsonl transcript files from a directory into candidates list.
fn collect_claude_jsonl(dir: &Path, candidates: &mut Vec<(SystemTime, String, PathBuf)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("claude")
                    .to_string();
                let mtime = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH);
                candidates.push((mtime, stem, p));
            }
        }
    }
}

/// Find the latest Claude Code session log.
pub fn find_latest_claude_log(repo_root: Option<&Path>) -> Option<(String, PathBuf)> {
    let claude_dir = claude_config_dir()?;
    let projects_dir = claude_dir.join("projects");

    let mut candidates: Vec<(SystemTime, String, PathBuf)> = Vec::new();

    if let Some(root) = repo_root {
        let encoded = encode_claude_project_path(root);
        let project_folder = projects_dir.join(&encoded);
        if project_folder.is_dir() {
            collect_claude_jsonl(&project_folder, &mut candidates);
            collect_claude_jsonl(&project_folder.join("sessions"), &mut candidates);
        }

        // Also check history.jsonl for sessions matching repo_root
        let history_file = claude_dir.join("history.jsonl");
        if let Ok(file) = fs::File::open(&history_file) {
            let root_str = root.to_string_lossy();
            let reader = std::io::BufReader::new(file);
            let mut matching_sessions = Vec::new();
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(val) = serde_json::from_str::<Value>(&line)
                    && let Some(proj) = val.get("project").and_then(Value::as_str)
                    && (proj == root_str.as_ref()
                        || proj.starts_with(root_str.as_ref())
                        || root_str.starts_with(proj))
                    && let Some(sess_id) = val.get("sessionId").and_then(Value::as_str)
                {
                    matching_sessions.push(sess_id.to_string());
                }
            }

            for sess in matching_sessions.iter().rev() {
                if let Some((name, p)) = resolve_claude_session_in(&claude_dir, sess) {
                    let mtime = fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH);
                    candidates.push((mtime, name, p));
                }
            }
        }
    }

    if candidates.is_empty()
        && projects_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&projects_dir)
    {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                collect_claude_jsonl(&entry.path(), &mut candidates);
                collect_claude_jsonl(&entry.path().join("sessions"), &mut candidates);
            }
        }
    }

    let sessions_dir = claude_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_claude_jsonl(&sessions_dir, &mut candidates);
    }

    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
    candidates.dedup_by(|a, b| a.2 == b.2);

    if candidates.is_empty() {
        return None;
    }

    let (_, name, log_path) = candidates.remove(0);
    Some((name, log_path))
}

/// Extract visible text from a Claude Code message payload.
fn extract_claude_text(value: &Value, is_user: bool) -> Option<String> {
    let message = value.get("message").unwrap_or(value);
    let content = message.get("content")?;

    match content {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(blocks) => {
            let mut texts = Vec::new();
            for block in blocks {
                let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
                if block_type == "text" || (block_type.is_empty() && block.get("text").is_some()) {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        let trimmed = t.trim();
                        if !trimmed.is_empty() {
                            texts.push(trimmed);
                        }
                    }
                } else if is_user && block_type == "tool_result" {
                    // Tool execution results in user turns are not prompts
                }
            }

            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        _ => None,
    }
}

/// Parse a Claude Code transcript from a reader.
pub fn parse_claude_reader<R: BufRead>(
    reader: R,
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let mut latest_prompt: Option<String> = None;
    let mut last_message: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let kind = value
            .get("type")
            .or_else(|| value.get("role"))
            .and_then(Value::as_str)
            .unwrap_or("");

        if kind == "user" {
            if let Some(text) = extract_claude_text(&value, true)
                && let Some(first) = first_line(&text)
            {
                let cleaned = clean_prompt_line(&first);
                if !cleaned.is_empty() {
                    latest_prompt = Some(sanitize_file_urls(&cleaned));
                }
            }
        } else if kind == "assistant"
            && let Some(text) = extract_claude_text(&value, false)
        {
            last_message = Some(sanitize_file_urls(&text));
        }
    }

    let raw_body = last_message.ok_or_else(|| {
        format!("Claude Code session `{session_id}` has no assistant response text to copy")
    })?;

    let sanitized_body = sanitize_chat_body(&raw_body, repo_root);

    let (subject, subject_source) = match latest_prompt {
        Some(prompt) => {
            let sanitized = sanitize_current_path(&sanitize_file_urls(&prompt), repo_root);
            (
                truncate(&sanitized, FALLBACK_SUBJECT_MAX),
                SubjectSource::LatestPrompt,
            )
        }
        None => {
            let line = first_line(&sanitized_body).unwrap_or_else(|| sanitized_body.clone());
            let sanitized = sanitize_current_path(&sanitize_file_urls(&line), repo_root);
            (
                truncate(&sanitized, FALLBACK_SUBJECT_MAX),
                SubjectSource::LastMessage,
            )
        }
    };

    Ok(ChatMessage {
        session_id: session_id.to_string(),
        subject: sanitize_file_urls(&subject),
        body: sanitized_body,
        subject_source,
    })
}

/// Parse a Claude Code transcript from a byte slice.
pub fn parse_claude(transcript: &[u8], session_id: &str) -> Result<ChatMessage, String> {
    parse_claude_reader(std::io::Cursor::new(transcript), session_id, None)
}

/// Load and parse a Claude Code transcript from disk.
pub fn load_claude(
    path: &Path,
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "could not open Claude Code transcript {}: {error}",
            path.display()
        )
    })?;
    let reader = std::io::BufReader::new(file);
    parse_claude_reader(reader, session_id, repo_root)
}

/// Helper struct that holds an open reader and optional child process (for zstd).
pub struct CodexFileStream {
    reader: Box<dyn BufRead>,
    child: Option<std::process::Child>,
}

impl BufRead for CodexFileStream {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.reader.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.reader.consume(amt);
    }
}

impl std::io::Read for CodexFileStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Drop for CodexFileStream {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Open a Codex session file, decompressing with `zstd` / `zstdcat` if `.zst`.
pub fn open_codex_file(path: &Path) -> Result<CodexFileStream, String> {
    let is_zst = path.extension().and_then(|s| s.to_str()) == Some("zst");

    if is_zst {
        let spawn_res = Command::new("zstd")
            .args(["-dc"])
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .or_else(|_| {
                Command::new("zstdcat")
                    .arg(path)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
            });

        match spawn_res {
            Ok(mut child) => {
                let stdout = child.stdout.take().ok_or_else(|| {
                    format!("failed to capture stdout from zstd for {}", path.display())
                })?;
                Ok(CodexFileStream {
                    reader: Box::new(std::io::BufReader::new(stdout)),
                    child: Some(child),
                })
            }
            Err(e) => Err(format!(
                "could not decompress Codex session {}: zstd or zstdcat command failed ({e})",
                path.display()
            )),
        }
    } else {
        let file = fs::File::open(path)
            .map_err(|error| format!("could not open Codex session {}: {error}", path.display()))?;
        Ok(CodexFileStream {
            reader: Box::new(std::io::BufReader::new(file)),
            child: None,
        })
    }
}

/// Quick peek at the first few lines of a Codex transcript to extract (session_id, cwd).
pub fn peek_codex_meta(path: &Path) -> Option<(Option<String>, Option<String>)> {
    let mut stream = open_codex_file(path).ok()?;
    let mut line = String::new();
    for _ in 0..5 {
        line.clear();
        if stream.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(trimmed)
            && val.get("type").and_then(Value::as_str) == Some("session_meta")
        {
            let id = val
                .pointer("/payload/id")
                .or_else(|| val.pointer("/payload/session_id"))
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let cwd = val
                .pointer("/payload/cwd")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            return Some((id, cwd));
        }
    }
    None
}

/// Path to Codex's home directory.
pub fn codex_home_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(CODEX_HOME_ENV) {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            return Some(path);
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let codex = home.join(".codex");
    if codex.is_dir() { Some(codex) } else { None }
}

/// Load thread_name for a given session ID from `session_index.jsonl` in the Codex home dir.
pub fn load_codex_thread_name(codex_dir: &Path, session_id: &str) -> Option<String> {
    let index_file = codex_dir.join("session_index.jsonl");
    let file = fs::File::open(index_file).ok()?;
    let reader = std::io::BufReader::new(file);

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(trimmed)
            && let Some(id) = val.get("id").and_then(Value::as_str)
            && (id == session_id || session_id.contains(id) || id.contains(session_id))
            && let Some(name) = val.get("thread_name").and_then(Value::as_str)
        {
            let trimmed_name = name.trim();
            if !trimmed_name.is_empty() && !is_placeholder_title(trimmed_name) {
                return Some(trimmed_name.to_string());
            }
        }
    }

    None
}

fn collect_codex_files_rec(dir: &Path, depth: usize, candidates: &mut Vec<(SystemTime, PathBuf)>) {
    if depth > 6 {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.ends_with(".jsonl") || name.ends_with(".jsonl.zst") {
                    let mtime = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH);
                    candidates.push((mtime, path));
                }
            } else if path.is_dir() {
                collect_codex_files_rec(&path, depth + 1, candidates);
            }
        }
    }
}

/// Recursively collect all `.jsonl` and `.jsonl.zst` files from a Codex directory.
pub fn collect_codex_files(dir: &Path, candidates: &mut Vec<(SystemTime, PathBuf)>) {
    collect_codex_files_rec(dir, 0, candidates);
}

/// Resolve a Codex session by session ID or file path.
pub fn resolve_codex_session(session: &str) -> Option<(String, PathBuf)> {
    let path = Path::new(session);
    if path.is_file() {
        let name = peek_codex_meta(path)
            .and_then(|(id, _)| id)
            .or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "codex".to_string());
        return Some((name, path.to_path_buf()));
    }
    if path.is_dir() {
        let mut candidates = Vec::new();
        collect_codex_files(path, &mut candidates);
        candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        if let Some((_, p)) = candidates.into_iter().next() {
            let name = peek_codex_meta(&p)
                .and_then(|(id, _)| id)
                .or_else(|| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "codex".to_string());
            return Some((name, p));
        }
    }

    let codex_dir = codex_home_dir()?;
    let mut candidates = Vec::new();

    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_codex_files(&sessions_dir, &mut candidates);
    }
    collect_codex_files(&codex_dir, &mut candidates);

    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
    candidates.dedup_by(|a, b| a.1 == b.1);

    // Look for exact ID match or filename containing session
    for (_, candidate_path) in &candidates {
        let fname = candidate_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if fname.contains(session) {
            let id = peek_codex_meta(candidate_path)
                .and_then(|(id, _)| id)
                .unwrap_or_else(|| session.to_string());
            return Some((id, candidate_path.clone()));
        }
    }

    // Check session_index.jsonl for matching id
    let index_file = codex_dir.join("session_index.jsonl");
    if let Ok(file) = fs::File::open(index_file) {
        let reader = std::io::BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(trimmed)
                && let Some(id) = val.get("id").and_then(Value::as_str)
                && (id == session || id.starts_with(session) || session.starts_with(id))
            {
                for (_, candidate_path) in &candidates {
                    let fname = candidate_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    if fname.contains(id) {
                        return Some((id.to_string(), candidate_path.clone()));
                    }
                }
            }
        }
    }

    None
}

/// Find the latest Codex session log, prioritizing sessions matching `repo_root`.
pub fn find_latest_codex_log(repo_root: Option<&Path>) -> Option<(String, PathBuf)> {
    let codex_dir = codex_home_dir()?;
    let mut candidates = Vec::new();

    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_codex_files(&sessions_dir, &mut candidates);
    }
    collect_codex_files(&codex_dir, &mut candidates);

    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
    candidates.dedup_by(|a, b| a.1 == b.1);

    if candidates.is_empty() {
        return None;
    }

    if let Some(root) = repo_root {
        let root_str = root.to_string_lossy();
        for (_, path) in &candidates {
            if let Some((id_opt, Some(cwd))) = peek_codex_meta(path)
                && (cwd == root_str.as_ref()
                    || cwd.starts_with(root_str.as_ref())
                    || root_str.starts_with(&cwd))
            {
                let name = id_opt.unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("codex")
                        .to_string()
                });
                return Some((name, path.clone()));
            }
        }
    }

    let (_, path) = candidates.remove(0);
    let name = peek_codex_meta(&path)
        .and_then(|(id, _)| id)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("codex")
                .to_string()
        });
    Some((name, path))
}

/// Extract text content from a Codex message payload or AgentMessage item.
fn extract_codex_text(value: &Value) -> Option<String> {
    if let Some(content) = value.get("content") {
        match content {
            Value::String(s) => {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            Value::Array(items) => {
                let mut parts = Vec::new();
                for item in items {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            parts.push(trimmed);
                        }
                    } else if let Some(s) = item.as_str() {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            parts.push(trimmed);
                        }
                    }
                }
                if !parts.is_empty() {
                    return Some(parts.join("\n"));
                }
            }
            _ => {}
        }
    }
    if let Some(msg) = value.get("last_agent_message").and_then(Value::as_str) {
        let trimmed = msg.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Parse a Codex session transcript from a reader.
pub fn parse_codex_reader<R: BufRead>(
    reader: R,
    session_id: &str,
    thread_name: Option<&str>,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let mut latest_prompt: Option<String> = None;
    let mut last_message: Option<String> = None;
    let mut inferred_session_id: Option<String> = None;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");

        if kind == "session_meta" {
            if inferred_session_id.is_none() {
                inferred_session_id = value
                    .pointer("/payload/id")
                    .or_else(|| value.pointer("/payload/session_id"))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
            }
        } else if kind == "response_item" {
            let payload = match value.get("payload") {
                Some(p) => p,
                None => continue,
            };
            let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
            if payload_type == "message" {
                let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
                if role == "user" {
                    if let Some(text) = extract_codex_text(payload)
                        && let Some(first) = first_line(&text)
                    {
                        let cleaned = clean_prompt_line(&first);
                        if !cleaned.is_empty() {
                            latest_prompt = Some(sanitize_file_urls(&cleaned));
                        }
                    }
                } else if role == "assistant"
                    && let Some(text) = extract_codex_text(payload)
                {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        last_message = Some(sanitize_file_urls(trimmed));
                    }
                }
            }
        } else if kind == "event_msg" {
            let payload = match value.get("payload") {
                Some(p) => p,
                None => continue,
            };
            let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
            if payload_type == "task_complete" {
                if let Some(msg) = payload.get("last_agent_message").and_then(Value::as_str) {
                    let trimmed = msg.trim();
                    if !trimmed.is_empty() {
                        last_message = Some(sanitize_file_urls(trimmed));
                    }
                }
            } else if payload_type == "item_completed"
                && let Some(item) = payload.get("item")
                && item.get("type").and_then(Value::as_str) == Some("AgentMessage")
                && let Some(text) = extract_codex_text(item)
            {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    last_message = Some(sanitize_file_urls(trimmed));
                }
            }
        }
    }

    let final_session_id = if session_id.is_empty() || session_id == "codex" {
        inferred_session_id.as_deref().unwrap_or(session_id)
    } else {
        session_id
    };

    let raw_body = last_message.ok_or_else(|| {
        format!("Codex session `{final_session_id}` has no assistant response text to copy")
    })?;

    let sanitized_body = sanitize_chat_body(&raw_body, repo_root);

    let (subject, subject_source) = if let Some(title) = thread_name
        .map(str::trim)
        .filter(|t| !t.is_empty() && !is_placeholder_title(t))
    {
        let sanitized = sanitize_current_path(&sanitize_file_urls(title), repo_root);
        (sanitized, SubjectSource::Title)
    } else if let Some(prompt) = latest_prompt {
        let sanitized = sanitize_current_path(&sanitize_file_urls(&prompt), repo_root);
        (
            truncate(&sanitized, FALLBACK_SUBJECT_MAX),
            SubjectSource::LatestPrompt,
        )
    } else {
        let line = first_line(&sanitized_body).unwrap_or_else(|| sanitized_body.clone());
        let sanitized = sanitize_current_path(&sanitize_file_urls(&line), repo_root);
        (
            truncate(&sanitized, FALLBACK_SUBJECT_MAX),
            SubjectSource::LastMessage,
        )
    };

    Ok(ChatMessage {
        session_id: final_session_id.to_string(),
        subject: sanitize_file_urls(&subject),
        body: sanitized_body,
        subject_source,
    })
}

/// Parse a Codex session transcript from a byte slice.
pub fn parse_codex(
    transcript: &[u8],
    session_id: &str,
    thread_name: Option<&str>,
) -> Result<ChatMessage, String> {
    parse_codex_reader(
        std::io::Cursor::new(transcript),
        session_id,
        thread_name,
        None,
    )
}

/// Load and parse a Codex transcript from disk.
pub fn load_codex(
    path: &Path,
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let codex_dir = codex_home_dir();
    let thread_name = codex_dir
        .as_deref()
        .and_then(|dir| load_codex_thread_name(dir, session_id))
        .or_else(|| {
            let (real_id, _) = peek_codex_meta(path)?;
            let id = real_id?;
            codex_dir
                .as_deref()
                .and_then(|dir| load_codex_thread_name(dir, &id))
        });

    let stream = open_codex_file(path)?;
    parse_codex_reader(stream, session_id, thread_name.as_deref(), repo_root)
}

/// Check if a role or message type indicates user input for Devin / Codeium.
fn is_devin_user_role(role_or_type: &str) -> bool {
    matches!(
        role_or_type.to_ascii_lowercase().as_str(),
        "user" | "human" | "user_input" | "user_message" | "prompt"
    )
}

/// Check if a role or message type indicates assistant output for Devin / Codeium.
fn is_devin_assistant_role(role_or_type: &str) -> bool {
    matches!(
        role_or_type.to_ascii_lowercase().as_str(),
        "assistant"
            | "agent"
            | "model"
            | "devin"
            | "cascade"
            | "agent_message"
            | "agent_response"
            | "response"
    )
}

/// Extract visible text from a Devin or Cascade message payload.
fn extract_devin_text(value: &Value, is_user: bool) -> Option<String> {
    let target = value.get("chat_message").unwrap_or(value);
    let content = target
        .get("content")
        .or_else(|| target.get("text"))
        .or_else(|| target.get("message"))
        .or_else(|| target.get("response"))?;

    match content {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Object(map) => {
            if let Some(text) = map
                .get("text")
                .or_else(|| map.get("value"))
                .and_then(Value::as_str)
            {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            None
        }
        Value::Array(blocks) => {
            let mut texts = Vec::new();
            for block in blocks {
                match block {
                    Value::String(s) => {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            texts.push(trimmed.to_string());
                        }
                    }
                    Value::Object(obj) => {
                        let block_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
                        if is_user && (block_type == "tool_result" || block_type == "tool_response")
                        {
                            continue;
                        }
                        if (block_type == "text"
                            || block_type == "message"
                            || block_type.is_empty())
                            && let Some(t) = obj
                                .get("text")
                                .or_else(|| obj.get("content"))
                                .and_then(Value::as_str)
                        {
                            let trimmed = t.trim();
                            if !trimmed.is_empty() {
                                texts.push(trimmed.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n\n"))
            }
        }
        _ => None,
    }
}

fn strip_thinking_blocks(text: &str) -> String {
    let mut in_thinking = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Thinking Process:")
            || trimmed.starts_with("Thinking:")
            || trimmed.starts_with("<details><summary>Thinking")
        {
            in_thinking = true;
            continue;
        }
        if in_thinking {
            if trimmed.starts_with("</details>") {
                in_thinking = false;
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                continue;
            } else {
                in_thinking = false;
            }
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// Parse a Markdown trajectory (such as Windsurf Cascade exported trajectory).
fn parse_devin_markdown(content: &str) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let mut title: Option<String> = None;
    let mut latest_prompt: Option<String> = None;
    let mut last_message: Option<String> = None;

    let mut current_section = "";
    let mut section_content = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if (trimmed.starts_with("# ") || trimmed.starts_with("## "))
            && title.is_none()
            && (trimmed.contains("Trajectory")
                || trimmed.contains("Cascade")
                || trimmed.contains("Devin"))
        {
            let t = trimmed.trim_start_matches('#').trim();
            if let Some(idx) = t.find(':') {
                let after = t[idx + 1..].trim();
                if !after.is_empty() && !is_placeholder_title(after) {
                    title = Some(after.to_string());
                }
            } else if !is_placeholder_title(t) {
                title = Some(t.to_string());
            }
        }

        let is_user_heading = trimmed.eq_ignore_ascii_case("## User")
            || trimmed.eq_ignore_ascii_case("### User")
            || trimmed.eq_ignore_ascii_case("# User")
            || trimmed == "**User**"
            || trimmed.starts_with("**User:**")
            || trimmed.starts_with("**User**:")
            || trimmed.starts_with("User:");

        let is_assistant_heading = trimmed.eq_ignore_ascii_case("## Cascade")
            || trimmed.eq_ignore_ascii_case("### Cascade")
            || trimmed.eq_ignore_ascii_case("# Cascade")
            || trimmed == "**Cascade**"
            || trimmed.starts_with("**Cascade:**")
            || trimmed.starts_with("**Cascade**:")
            || trimmed.starts_with("Cascade:")
            || trimmed.eq_ignore_ascii_case("## Assistant")
            || trimmed.eq_ignore_ascii_case("### Assistant")
            || trimmed.eq_ignore_ascii_case("# Assistant")
            || trimmed == "**Assistant**"
            || trimmed.starts_with("**Assistant:**")
            || trimmed.starts_with("**Assistant**:")
            || trimmed.starts_with("Assistant:")
            || trimmed.eq_ignore_ascii_case("## Devin")
            || trimmed.eq_ignore_ascii_case("### Devin")
            || trimmed.eq_ignore_ascii_case("# Devin")
            || trimmed == "**Devin**"
            || trimmed.starts_with("**Devin:**")
            || trimmed.starts_with("**Devin**:")
            || trimmed.starts_with("Devin:");

        if is_user_heading {
            if current_section == "assistant" && !section_content.trim().is_empty() {
                let stripped = strip_thinking_blocks(section_content.trim());
                if !stripped.is_empty() {
                    last_message = Some(stripped);
                }
            }
            current_section = "user";
            section_content.clear();
            if let Some(idx) = trimmed.find(':') {
                section_content.push_str(trimmed[idx + 1..].trim());
                section_content.push('\n');
            }
        } else if is_assistant_heading {
            if current_section == "user"
                && !section_content.trim().is_empty()
                && let Some(first) = first_line(section_content.trim())
            {
                let cleaned = clean_prompt_line(&first);
                if !cleaned.is_empty() {
                    latest_prompt = Some(cleaned);
                }
            }
            current_section = "assistant";
            section_content.clear();
            if let Some(idx) = trimmed.find(':') {
                section_content.push_str(trimmed[idx + 1..].trim());
                section_content.push('\n');
            }
        } else if !current_section.is_empty() {
            section_content.push_str(line);
            section_content.push('\n');
        }
    }

    if current_section == "user"
        && !section_content.trim().is_empty()
        && let Some(first) = first_line(section_content.trim())
    {
        let cleaned = clean_prompt_line(&first);
        if !cleaned.is_empty() {
            latest_prompt = Some(cleaned);
        }
    } else if current_section == "assistant" && !section_content.trim().is_empty() {
        let stripped = strip_thinking_blocks(section_content.trim());
        if !stripped.is_empty() {
            last_message = Some(stripped);
        }
    }

    if last_message.is_some() {
        Some((title, latest_prompt, last_message))
    } else {
        None
    }
}

/// Return candidate transcript directories for Devin and Codeium / Cascade.
pub fn devin_transcripts_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(dir) = std::env::var(DEVIN_HOME_ENV) {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            dirs.push(p.join("transcripts"));
            dirs.push(p.join("cli").join("transcripts"));
            dirs.push(p.join("cascade"));
            dirs.push(p.join("windsurf").join("cascade"));
            dirs.push(p.clone());
        }
    }

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(
            home.join(".local")
                .join("share")
                .join("devin")
                .join("cli")
                .join("transcripts"),
        );
        dirs.push(
            home.join(".local")
                .join("share")
                .join("devin")
                .join("transcripts"),
        );
        dirs.push(home.join(".config").join("devin").join("transcripts"));
        dirs.push(home.join(".devin").join("transcripts"));
        dirs.push(home.join(".devin").join("sessions"));
        dirs.push(home.join(".devin"));
        dirs.push(home.join(".codeium").join("windsurf").join("cascade"));
    }

    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        dirs.push(appdata.join("devin").join("cli").join("transcripts"));
        dirs.push(appdata.join("devin").join("transcripts"));
        dirs.push(appdata.join("Codeium").join("windsurf").join("cascade"));
    }

    dirs.retain(|d| d.is_dir());
    dirs
}

/// Recursively collect Devin / Cascade transcript files from a directory into candidates list.
fn collect_devin_transcripts_rec(
    dir: &Path,
    depth: usize,
    candidates: &mut Vec<(SystemTime, String, PathBuf)>,
) {
    if depth > 4 {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
                if matches!(ext, "json" | "jsonl" | "atif" | "md") {
                    let stem = p
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("devin")
                        .to_string();
                    let mtime = fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH);
                    candidates.push((mtime, stem, p));
                }
            } else if p.is_dir() {
                collect_devin_transcripts_rec(&p, depth + 1, candidates);
            }
        }
    }
}

/// Collect Devin / Cascade transcript files from a directory into candidates list.
pub fn collect_devin_transcripts(dir: &Path, candidates: &mut Vec<(SystemTime, String, PathBuf)>) {
    collect_devin_transcripts_rec(dir, 0, candidates);
}

/// Resolve a Devin session inside a directory.
pub fn resolve_devin_session_in(dir: &Path, session: &str) -> Option<(String, PathBuf)> {
    for ext in ["json", "jsonl", "atif", "md"] {
        let direct = dir.join(format!("{session}.{ext}"));
        if direct.is_file() {
            return Some((session.to_string(), direct));
        }
    }

    let exact = dir.join(session);
    if exact.is_file() {
        return Some((session.to_string(), exact));
    }

    let mut candidates = Vec::new();
    collect_devin_transcripts(dir, &mut candidates);
    for (_, name, p) in candidates {
        if name == session || name.starts_with(session) || session.starts_with(&name) {
            return Some((name, p));
        }
    }

    None
}

/// Resolve a Devin / Cascade session by session ID or file path.
pub fn resolve_devin_session(session: &str) -> Option<(String, PathBuf)> {
    let path = Path::new(session);
    if path.is_file() {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("devin")
            .to_string();
        return Some((name, path.to_path_buf()));
    }
    if path.is_dir() {
        let mut candidates = Vec::new();
        collect_devin_transcripts(path, &mut candidates);
        candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        if let Some((_, name, p)) = candidates.into_iter().next() {
            return Some((name, p));
        }
    }

    for dir in devin_transcripts_dirs() {
        if let Some(resolved) = resolve_devin_session_in(&dir, session) {
            return Some(resolved);
        }
    }

    None
}

/// Find the latest Devin / Cascade session log.
pub fn find_latest_devin_log(repo_root: Option<&Path>) -> Option<(String, PathBuf)> {
    let mut candidates = Vec::new();
    for dir in devin_transcripts_dirs() {
        collect_devin_transcripts(&dir, &mut candidates);
    }

    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
    candidates.dedup_by(|a, b| a.2 == b.2);

    if candidates.is_empty() {
        return None;
    }

    if let Some(root) = repo_root {
        let root_str = root.to_string_lossy();
        for (_, name, log_path) in candidates.iter().take(20) {
            if let Ok(mut file) = fs::File::open(log_path) {
                use std::io::Read;
                let mut buffer = [0u8; 65536];
                if let Ok(n) = file.read(&mut buffer) {
                    let sample = String::from_utf8_lossy(&buffer[..n]);
                    if sample.contains(&*root_str) {
                        return Some((name.clone(), log_path.clone()));
                    }
                }
            }
        }
    }

    let (_, name, log_path) = candidates.remove(0);
    Some((name, log_path))
}

/// Parse a Devin transcript from a reader.
pub fn parse_devin_reader<R: BufRead>(
    mut reader: R,
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let mut buffer = String::new();
    reader
        .read_to_string(&mut buffer)
        .map_err(|e| format!("failed to read transcript: {e}"))?;

    let trimmed = buffer.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "Devin session `{session_id}` has no assistant response text to copy"
        ));
    }

    let mut latest_prompt: Option<String> = None;
    let mut last_message: Option<String> = None;
    let mut title: Option<String> = None;
    let mut inferred_session_id: Option<String> = None;

    // 1. Try parsing as full JSON (ATIF JSON, JSON session export)
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Ok(root_val) = serde_json::from_str::<Value>(trimmed)
    {
        if let Some(id) = root_val
            .get("session_id")
            .or_else(|| root_val.get("id"))
            .and_then(Value::as_str)
        {
            inferred_session_id = Some(id.to_string());
        }
        if let Some(t) = root_val
            .get("title")
            .or_else(|| root_val.get("session_name"))
            .or_else(|| root_val.get("name"))
            .and_then(Value::as_str)
        {
            let t_trimmed = t.trim();
            if !t_trimmed.is_empty() && !is_placeholder_title(t_trimmed) {
                title = Some(t_trimmed.to_string());
            }
        }

        let items: Option<&Vec<Value>> = root_val
            .get("steps")
            .or_else(|| root_val.get("messages"))
            .or_else(|| root_val.get("nodes"))
            .or_else(|| root_val.get("trajectory"))
            .or_else(|| root_val.get("history"))
            .or_else(|| root_val.get("turns"))
            .and_then(Value::as_array)
            .or_else(|| root_val.as_array());

        if let Some(steps) = items {
            for step in steps {
                let role = step
                    .get("role")
                    .or_else(|| step.get("type"))
                    .or_else(|| step.get("source"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let is_user_flag = step
                    .get("is_user_input")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

                if is_user_flag || is_devin_user_role(role) {
                    if let Some(text) = extract_devin_text(step, true) {
                        let cleaned_text = extract_user_request(&text);
                        if let Some(first) = first_line(&cleaned_text) {
                            let cleaned = clean_prompt_line(&first);
                            if !cleaned.is_empty() {
                                latest_prompt = Some(sanitize_file_urls(&cleaned));
                            }
                        }
                    }
                } else if is_devin_assistant_role(role)
                    && let Some(text) = extract_devin_text(step, false)
                {
                    let trimmed_text = text.trim();
                    if !trimmed_text.is_empty() {
                        last_message = Some(sanitize_file_urls(trimmed_text));
                    }
                }
            }
        }
    }

    // 2. If no assistant message found, try parsing as JSON Lines
    if last_message.is_none() {
        for line in trimmed.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(line_trimmed) {
                if inferred_session_id.is_none()
                    && let Some(id) = val
                        .get("session_id")
                        .or_else(|| val.get("id"))
                        .and_then(Value::as_str)
                {
                    inferred_session_id = Some(id.to_string());
                }
                let role = val
                    .get("role")
                    .or_else(|| val.get("type"))
                    .or_else(|| val.get("source"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let is_user_flag = val
                    .get("is_user_input")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

                if is_user_flag || is_devin_user_role(role) {
                    if let Some(text) = extract_devin_text(&val, true) {
                        let cleaned_text = extract_user_request(&text);
                        if let Some(first) = first_line(&cleaned_text) {
                            let cleaned = clean_prompt_line(&first);
                            if !cleaned.is_empty() {
                                latest_prompt = Some(sanitize_file_urls(&cleaned));
                            }
                        }
                    }
                } else if is_devin_assistant_role(role)
                    && let Some(text) = extract_devin_text(&val, false)
                {
                    let trimmed_text = text.trim();
                    if !trimmed_text.is_empty() {
                        last_message = Some(sanitize_file_urls(trimmed_text));
                    }
                }
            }
        }
    }

    // 3. If still no messages found, try Markdown format
    if last_message.is_none()
        && let Some((md_title, md_prompt, md_msg)) = parse_devin_markdown(trimmed)
    {
        if title.is_none() {
            title = md_title;
        }
        if latest_prompt.is_none() {
            latest_prompt = md_prompt;
        }
        last_message = md_msg;
    }

    let final_session_id =
        if session_id.is_empty() || session_id == "devin" || session_id == "cascade" {
            inferred_session_id.as_deref().unwrap_or(session_id)
        } else {
            session_id
        };

    let raw_body = last_message.ok_or_else(|| {
        format!("Devin session `{final_session_id}` has no assistant response text to copy")
    })?;

    let sanitized_body = sanitize_chat_body(&raw_body, repo_root);

    let (subject, subject_source) = if let Some(t) = title {
        let sanitized = sanitize_current_path(&sanitize_file_urls(&t), repo_root);
        (sanitized, SubjectSource::Title)
    } else if let Some(prompt) = latest_prompt {
        let sanitized = sanitize_current_path(&sanitize_file_urls(&prompt), repo_root);
        (
            truncate(&sanitized, FALLBACK_SUBJECT_MAX),
            SubjectSource::LatestPrompt,
        )
    } else {
        let line = first_line(&sanitized_body).unwrap_or_else(|| sanitized_body.clone());
        let sanitized = sanitize_current_path(&sanitize_file_urls(&line), repo_root);
        (
            truncate(&sanitized, FALLBACK_SUBJECT_MAX),
            SubjectSource::LastMessage,
        )
    };

    Ok(ChatMessage {
        session_id: final_session_id.to_string(),
        subject: sanitize_file_urls(&subject),
        body: sanitized_body,
        subject_source,
    })
}

/// Parse a Devin transcript from a byte slice.
pub fn parse_devin(transcript: &[u8], session_id: &str) -> Result<ChatMessage, String> {
    parse_devin_reader(std::io::Cursor::new(transcript), session_id, None)
}

/// Load and parse a Devin transcript from disk.
pub fn load_devin(
    path: &Path,
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "could not open Devin transcript {}: {error}",
            path.display()
        )
    })?;
    let reader = std::io::BufReader::new(file);
    parse_devin_reader(reader, session_id, repo_root)
}

/// Return candidate configuration / data directories for Freebuff and Codebuff.
pub fn freebuff_config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    for env_var in [
        FREEBUFF_CONFIG_DIR_ENV,
        CODEBUFF_CONFIG_DIR_ENV,
        FREEBUFF_HOME_ENV,
        CODEBUFF_HOME_ENV,
    ] {
        if let Ok(dir) = std::env::var(env_var) {
            let p = PathBuf::from(dir);
            if p.is_dir() {
                dirs.push(p);
            }
        }
    }

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".config").join("manicode"));
        dirs.push(home.join(".config").join("freebuff"));
        dirs.push(home.join(".config").join("codebuff"));
        dirs.push(home.join(".freebuff"));
        dirs.push(home.join(".codebuff"));
        dirs.push(home.join(".manicode"));
    }

    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        dirs.push(appdata.join("manicode"));
        dirs.push(appdata.join("freebuff"));
        dirs.push(appdata.join("codebuff"));
    }

    dirs.retain(|d| d.is_dir());
    dirs.dedup();
    dirs
}

fn collect_freebuff_candidates_in(
    config_dir: &Path,
    repo_root: Option<&Path>,
    project_candidates: &mut Vec<(SystemTime, String, PathBuf)>,
    other_candidates: &mut Vec<(SystemTime, String, PathBuf)>,
) {
    let project_name = repo_root
        .and_then(|r| r.file_name())
        .and_then(|s| s.to_str());

    let projects_dir = config_dir.join("projects");
    if let Ok(entries) = fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let entry_name = entry.file_name().to_string_lossy().to_string();
                let is_matching_project = project_name.map(|p| p == entry_name).unwrap_or(false);

                let chats_dir = entry.path().join("chats");
                if let Ok(chat_entries) = fs::read_dir(&chats_dir) {
                    for chat_entry in chat_entries.flatten() {
                        let chat_path = chat_entry.path();
                        if chat_path.is_dir() {
                            let msg_file = chat_path.join("chat-messages.json");
                            if msg_file.is_file() {
                                let id = chat_path
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("freebuff")
                                    .to_string();
                                let mtime = fs::metadata(&msg_file)
                                    .and_then(|m| m.modified())
                                    .or_else(|_| {
                                        fs::metadata(&chat_path).and_then(|m| m.modified())
                                    })
                                    .unwrap_or(UNIX_EPOCH);

                                if is_matching_project {
                                    project_candidates.push((mtime, id, msg_file));
                                } else {
                                    other_candidates.push((mtime, id, msg_file));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Direct chats dir under config_dir
    let chats_dir = config_dir.join("chats");
    if let Ok(chat_entries) = fs::read_dir(&chats_dir) {
        for chat_entry in chat_entries.flatten() {
            let chat_path = chat_entry.path();
            if chat_path.is_dir() {
                let msg_file = chat_path.join("chat-messages.json");
                if msg_file.is_file() {
                    let id = chat_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("freebuff")
                        .to_string();
                    let mtime = fs::metadata(&msg_file)
                        .and_then(|m| m.modified())
                        .or_else(|_| fs::metadata(&chat_path).and_then(|m| m.modified()))
                        .unwrap_or(UNIX_EPOCH);
                    other_candidates.push((mtime, id, msg_file));
                }
            }
        }
    }
}

/// Find the latest Freebuff / Codebuff session log inside a configuration directory.
pub fn find_latest_freebuff_log_in(
    config_dir: &Path,
    repo_root: Option<&Path>,
) -> Option<(String, PathBuf)> {
    let mut project_candidates = Vec::new();
    let mut other_candidates = Vec::new();
    collect_freebuff_candidates_in(
        config_dir,
        repo_root,
        &mut project_candidates,
        &mut other_candidates,
    );

    if !project_candidates.is_empty() {
        project_candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        let (_, id, path) = project_candidates.remove(0);
        return Some((id, path));
    }

    if !other_candidates.is_empty() {
        other_candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        let (_, id, path) = other_candidates.remove(0);
        return Some((id, path));
    }

    None
}

/// Find the latest Freebuff / Codebuff session log.
pub fn find_latest_freebuff_log(repo_root: Option<&Path>) -> Option<(String, PathBuf)> {
    let dirs = freebuff_config_dirs();
    let mut project_candidates: Vec<(SystemTime, String, PathBuf)> = Vec::new();
    let mut other_candidates: Vec<(SystemTime, String, PathBuf)> = Vec::new();

    for config_dir in &dirs {
        collect_freebuff_candidates_in(
            config_dir,
            repo_root,
            &mut project_candidates,
            &mut other_candidates,
        );
    }

    // Check inside repo root if .freebuff/chats or .codebuff/chats exists
    if let Some(root) = repo_root {
        for sub in [".freebuff", ".codebuff"] {
            let chats_dir = root.join(sub).join("chats");
            if let Ok(chat_entries) = fs::read_dir(&chats_dir) {
                for chat_entry in chat_entries.flatten() {
                    let chat_path = chat_entry.path();
                    if chat_path.is_dir() {
                        let msg_file = chat_path.join("chat-messages.json");
                        if msg_file.is_file() {
                            let id = chat_path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("freebuff")
                                .to_string();
                            let mtime = fs::metadata(&msg_file)
                                .and_then(|m| m.modified())
                                .unwrap_or(UNIX_EPOCH);
                            project_candidates.push((mtime, id, msg_file));
                        }
                    }
                }
            }
        }
    }

    if !project_candidates.is_empty() {
        project_candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        let (_, id, path) = project_candidates.remove(0);
        return Some((id, path));
    }

    if !other_candidates.is_empty() {
        other_candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
        let (_, id, path) = other_candidates.remove(0);
        return Some((id, path));
    }

    None
}

/// Resolve a Freebuff session inside a configuration directory.
pub fn resolve_freebuff_session_in(
    config_dir: &Path,
    session: &str,
    repo_root: Option<&Path>,
) -> Option<(String, PathBuf)> {
    let project_name = repo_root
        .and_then(|r| r.file_name())
        .and_then(|s| s.to_str());

    // 1. Check matching project first if repo_root is available
    if let Some(proj) = project_name {
        let chat_dir = config_dir
            .join("projects")
            .join(proj)
            .join("chats")
            .join(session);
        let msg_file = chat_dir.join("chat-messages.json");
        if msg_file.is_file() {
            return Some((session.to_string(), msg_file));
        }
    }

    // 2. Check all projects in config_dir
    let projects_dir = config_dir.join("projects");
    if let Ok(entries) = fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let chat_dir = entry.path().join("chats").join(session);
                let msg_file = chat_dir.join("chat-messages.json");
                if msg_file.is_file() {
                    return Some((session.to_string(), msg_file));
                }
            }
        }
    }

    // 3. Direct chats directory
    let chat_dir = config_dir.join("chats").join(session);
    let msg_file = chat_dir.join("chat-messages.json");
    if msg_file.is_file() {
        return Some((session.to_string(), msg_file));
    }

    // 4. Prefix or fuzzy matching across all chats
    if let Ok(entries) = fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let chats_dir = entry.path().join("chats");
                if let Ok(chat_entries) = fs::read_dir(&chats_dir) {
                    for chat_entry in chat_entries.flatten() {
                        let chat_path = chat_entry.path();
                        if chat_path.is_dir() {
                            let name = chat_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                            if name == session
                                || name.starts_with(session)
                                || session.starts_with(name)
                            {
                                let msg_file = chat_path.join("chat-messages.json");
                                if msg_file.is_file() {
                                    return Some((name.to_string(), msg_file));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Resolve a Freebuff session by session ID, chat directory, or file path.
pub fn resolve_freebuff_session(
    session: &str,
    repo_root: Option<&Path>,
) -> Option<(String, PathBuf)> {
    let path = Path::new(session);
    if path.is_file() {
        let name = if path
            .file_name()
            .map(|n| n == "chat-messages.json")
            .unwrap_or(false)
        {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("freebuff")
                .to_string()
        } else {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("freebuff")
                .to_string()
        };
        return Some((name, path.to_path_buf()));
    }
    if path.is_dir() {
        let direct = path.join("chat-messages.json");
        if direct.is_file() {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("freebuff")
                .to_string();
            return Some((name, direct));
        }
    }

    for dir in freebuff_config_dirs() {
        if let Some(resolved) = resolve_freebuff_session_in(&dir, session, repo_root) {
            return Some(resolved);
        }
    }

    // Check inside repo root if .freebuff/chats or .codebuff/chats exists
    if let Some(root) = repo_root {
        for sub in [".freebuff", ".codebuff"] {
            let chat_dir = root.join(sub).join("chats").join(session);
            let msg_file = chat_dir.join("chat-messages.json");
            if msg_file.is_file() {
                return Some((session.to_string(), msg_file));
            }
        }
    }

    None
}

/// Extract visible text content from a Freebuff / Codebuff assistant turn.
fn extract_freebuff_assistant_text(value: &Value) -> Option<String> {
    if let Some(blocks) = value.get("blocks").and_then(Value::as_array) {
        let mut texts = Vec::new();
        for block in blocks {
            let btype = block.get("type").and_then(Value::as_str).unwrap_or("");
            if btype == "text" {
                let text_type = block.get("textType").and_then(Value::as_str).unwrap_or("");
                if text_type == "reasoning" {
                    continue;
                }
                if let Some(content) = block
                    .get("content")
                    .or_else(|| block.get("text"))
                    .and_then(Value::as_str)
                {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        texts.push(trimmed.to_string());
                    }
                }
            } else if btype == "agent" {
                if let Some(agent_blocks) = block.get("blocks").and_then(Value::as_array) {
                    for ablock in agent_blocks {
                        if ablock.get("type").and_then(Value::as_str) == Some("text") {
                            let text_type =
                                ablock.get("textType").and_then(Value::as_str).unwrap_or("");
                            if text_type != "reasoning"
                                && let Some(content) = ablock
                                    .get("content")
                                    .or_else(|| ablock.get("text"))
                                    .and_then(Value::as_str)
                            {
                                let trimmed = content.trim();
                                if !trimmed.is_empty() {
                                    texts.push(trimmed.to_string());
                                }
                            }
                        }
                    }
                } else if let Some(content) = block.get("content").and_then(Value::as_str) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        texts.push(trimmed.to_string());
                    }
                }
            }
        }
        if !texts.is_empty() {
            return Some(texts.join("\n\n"));
        }
    }

    if let Some(content) = value
        .get("content")
        .or_else(|| value.get("text"))
        .and_then(Value::as_str)
    {
        let stripped = strip_xml_tag_block(content, "think");
        let stripped = strip_thinking_blocks(&stripped);
        let trimmed = stripped.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

/// Extract visible text content from a Freebuff / Codebuff user turn.
fn extract_freebuff_user_text(value: &Value) -> Option<String> {
    if let Some(content) = value
        .get("content")
        .or_else(|| value.get("text"))
        .and_then(Value::as_str)
    {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(blocks) = value.get("blocks").and_then(Value::as_array) {
        let mut texts = Vec::new();
        for block in blocks {
            let btype = block.get("type").and_then(Value::as_str).unwrap_or("");
            if (btype == "text" || btype.is_empty())
                && let Some(content) = block
                    .get("content")
                    .or_else(|| block.get("text"))
                    .and_then(Value::as_str)
            {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    texts.push(trimmed.to_string());
                }
            }
        }
        if !texts.is_empty() {
            return Some(texts.join("\n\n"));
        }
    }

    None
}

/// Parse a Freebuff / Codebuff transcript from a reader.
pub fn parse_freebuff_reader<R: BufRead>(
    mut reader: R,
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let mut buffer = String::new();
    reader
        .read_to_string(&mut buffer)
        .map_err(|e| format!("failed to read transcript: {e}"))?;

    let trimmed = buffer.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "Freebuff session `{session_id}` has no assistant response text to copy"
        ));
    }

    let mut latest_prompt: Option<String> = None;
    let mut last_message: Option<String> = None;
    let mut title: Option<String> = None;

    if (trimmed.starts_with('[') || trimmed.starts_with('{'))
        && let Ok(root_val) = serde_json::from_str::<Value>(trimmed)
    {
        if let Some(t) = root_val
            .get("title")
            .or_else(|| root_val.get("session_name"))
            .or_else(|| root_val.get("name"))
            .and_then(Value::as_str)
        {
            let t_trimmed = t.trim();
            if !t_trimmed.is_empty() && !is_placeholder_title(t_trimmed) {
                title = Some(t_trimmed.to_string());
            }
        }

        let items: Option<&Vec<Value>> = root_val
            .as_array()
            .or_else(|| root_val.get("messages").and_then(Value::as_array))
            .or_else(|| root_val.get("steps").and_then(Value::as_array))
            .or_else(|| root_val.get("history").and_then(Value::as_array));

        if let Some(messages) = items {
            for msg in messages {
                let variant = msg
                    .get("variant")
                    .or_else(|| msg.get("role"))
                    .or_else(|| msg.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");

                if variant == "user" {
                    if let Some(text) = extract_freebuff_user_text(msg) {
                        let cleaned_text = extract_user_request(&text);
                        if let Some(first) = first_line(&cleaned_text) {
                            let cleaned = clean_prompt_line(&first);
                            if !cleaned.is_empty() {
                                latest_prompt = Some(sanitize_file_urls(&cleaned));
                            }
                        }
                    }
                } else if (variant == "ai"
                    || variant == "assistant"
                    || variant == "agent"
                    || variant == "model")
                    && let Some(text) = extract_freebuff_assistant_text(msg)
                {
                    let trimmed_text = text.trim();
                    if !trimmed_text.is_empty() {
                        last_message = Some(sanitize_file_urls(trimmed_text));
                    }
                }
            }
        }
    }

    if last_message.is_none() {
        for line in trimmed.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<Value>(line_trimmed) {
                let variant = msg
                    .get("variant")
                    .or_else(|| msg.get("role"))
                    .or_else(|| msg.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");

                if variant == "user" {
                    if let Some(text) = extract_freebuff_user_text(&msg) {
                        let cleaned_text = extract_user_request(&text);
                        if let Some(first) = first_line(&cleaned_text) {
                            let cleaned = clean_prompt_line(&first);
                            if !cleaned.is_empty() {
                                latest_prompt = Some(sanitize_file_urls(&cleaned));
                            }
                        }
                    }
                } else if (variant == "ai"
                    || variant == "assistant"
                    || variant == "agent"
                    || variant == "model")
                    && let Some(text) = extract_freebuff_assistant_text(&msg)
                {
                    let trimmed_text = text.trim();
                    if !trimmed_text.is_empty() {
                        last_message = Some(sanitize_file_urls(trimmed_text));
                    }
                }
            }
        }
    }

    let raw_body = last_message.ok_or_else(|| {
        format!("Freebuff session `{session_id}` has no assistant response text to copy")
    })?;

    let sanitized_body = sanitize_chat_body(&raw_body, repo_root);

    let (subject, subject_source) = if let Some(t) = title {
        (truncate(&t, FALLBACK_SUBJECT_MAX), SubjectSource::Title)
    } else if let Some(prompt) = latest_prompt {
        let sanitized = sanitize_current_path(&sanitize_file_urls(&prompt), repo_root);
        (
            truncate(&sanitized, FALLBACK_SUBJECT_MAX),
            SubjectSource::LatestPrompt,
        )
    } else {
        let line = first_line(&sanitized_body).unwrap_or_else(|| sanitized_body.clone());
        let sanitized = sanitize_current_path(&sanitize_file_urls(&line), repo_root);
        (
            truncate(&sanitized, FALLBACK_SUBJECT_MAX),
            SubjectSource::LastMessage,
        )
    };

    Ok(ChatMessage {
        session_id: session_id.to_string(),
        subject: sanitize_file_urls(&subject),
        body: sanitized_body,
        subject_source,
    })
}

/// Parse a Freebuff / Codebuff transcript from a byte slice.
pub fn parse_freebuff(transcript: &[u8], session_id: &str) -> Result<ChatMessage, String> {
    parse_freebuff_reader(std::io::Cursor::new(transcript), session_id, None)
}

/// Load and parse a Freebuff / Codebuff transcript from disk.
pub fn load_freebuff(
    path: &Path,
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "could not open Freebuff transcript {}: {error}",
            path.display()
        )
    })?;
    let reader = std::io::BufReader::new(file);
    parse_freebuff_reader(reader, session_id, repo_root)
}

/// Load and parse a session export or conversation log.
///
/// Sources supported:
/// - An explicit `agent` (for example from `--opencode`).
/// - Explicit sessions via `session` and environment variables.
/// - Auto-detection of the latest local transcript across Antigravity, Claude
///   Code, Codex, Devin, and Freebuff.
pub fn load(
    session: Option<&str>,
    repo_root: Option<&Path>,
    agent: Option<Agent>,
    verbose: bool,
) -> Result<ChatMessage, String> {
    let mut chat = load_raw(session, repo_root, agent, verbose)?;
    chat.subject = sanitize_current_path(&chat.subject, repo_root);
    chat.body = sanitize_chat_body(&chat.body, repo_root);
    Ok(chat)
}

fn load_raw(
    session: Option<&str>,
    repo_root: Option<&Path>,
    agent: Option<Agent>,
    verbose: bool,
) -> Result<ChatMessage, String> {
    let session = session.map(str::trim).filter(|value| !value.is_empty());

    if let Some(agent) = agent {
        return load_agent(agent, session, repo_root, verbose);
    }

    if let Some(sess) = session {
        for agent in Agent::SESSION_ORDER {
            if let Some((id, path)) = agent.resolve(sess, repo_root) {
                announce(agent, &id, &path, verbose);
                return agent.parse(&path, &id, repo_root);
            }
        }

        return load_opencode(Some(sess), repo_root, verbose);
    }

    for agent in Agent::ENV_ORDER {
        if agent == Agent::OpenCode {
            if let Some(sess) = agent.env_session() {
                return load_opencode(Some(&sess), repo_root, verbose);
            }
            continue;
        }

        if let Some((id, path)) = agent
            .env_session()
            .and_then(|sess| agent.resolve(&sess, repo_root))
        {
            announce(agent, &id, &path, verbose);
            return agent.parse(&path, &id, repo_root);
        }
    }

    let mut candidates: Vec<(SystemTime, Agent, String, PathBuf)> = Agent::LOCAL
        .into_iter()
        .filter_map(|agent| {
            let (id, path) = agent.find_latest(repo_root)?;
            let mtime = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            Some((mtime, agent, id, path))
        })
        .collect();

    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));

    if let Some((_, agent, id, path)) = candidates.into_iter().next() {
        if verbose {
            eprintln!(
                "[mm] chat: auto-detected {} log {} ({})",
                agent.label(),
                id,
                path.display()
            );
        }
        return agent.parse(&path, &id, repo_root);
    }

    Err(format!(
        "no OpenCode session, Antigravity log, Claude Code session, Codex session, Devin log, or Freebuff session found; set {SESSION_ENV}, {ANTIGRAVITY_SESSION_ENV}, {CLAUDE_SESSION_ENV}, {CODEX_SESSION_ENV}, {DEVIN_SESSION_ENV}, {FREEBUFF_SESSION_ENV}, or pass --chat-session <ID>"
    ))
}

/// Load from a specific agent, whether forced by a flag or selected by default.
fn load_agent(
    agent: Agent,
    session: Option<&str>,
    repo_root: Option<&Path>,
    verbose: bool,
) -> Result<ChatMessage, String> {
    if agent == Agent::OpenCode {
        return load_opencode(session, repo_root, verbose);
    }

    let (id, path) = if let Some(sess) = session {
        agent.resolve(sess, repo_root).ok_or_else(|| {
            format!(
                "could not find {} session or transcript `{sess}`",
                agent.label()
            )
        })?
    } else if let Some(resolved) = agent
        .env_session()
        .and_then(|sess| agent.resolve(&sess, repo_root))
    {
        resolved
    } else {
        agent
            .find_latest(repo_root)
            .ok_or_else(|| format!("no {} session log found", agent.label()))?
    };

    announce(agent, &id, &path, verbose);
    agent.parse(&path, &id, repo_root)
}

/// Load the current OpenCode session through `opencode session export`.
fn load_opencode(
    session: Option<&str>,
    repo_root: Option<&Path>,
    verbose: bool,
) -> Result<ChatMessage, String> {
    let session = match session {
        Some(session) => session.to_string(),
        None => Agent::OpenCode.env_session().ok_or_else(|| {
            format!("no OpenCode session selected; set {SESSION_ENV} or pass --chat-session <ID>")
        })?,
    };

    if session.trim().is_empty() {
        return Err(format!(
            "no OpenCode session selected; set {SESSION_ENV} or pass --chat-session <ID>"
        ));
    }

    if verbose {
        eprintln!("[mm] chat: exporting OpenCode session {session}");
    }

    let export = export_session(&session)?;
    parse_with_root(&export, &session, repo_root)
}

/// Report which local transcript is being copied in verbose mode.
fn announce(agent: Agent, session_id: &str, path: &Path, verbose: bool) {
    if verbose {
        eprintln!(
            "[mm] chat: using {} log {} ({})",
            agent.label(),
            session_id,
            path.display()
        );
    }
}

/// Export a session to a temporary file.
///
/// `opencode session export` truncates output at 256 KiB when stdout is a
/// pipe, so the child writes directly to a file instead.
fn export_session(session: &str) -> Result<Vec<u8>, String> {
    let path = export_path();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let file = options.open(&path).map_err(|error| {
        format!("could not create a temporary file for the OpenCode export: {error}")
    })?;

    let result = (|| {
        let child = Command::new("opencode")
            .args(["session", "export", session])
            .stdout(Stdio::from(file))
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run `opencode session export`: {error}"))?;

        let output = child
            .wait_with_output()
            .map_err(|error| format!("could not read `opencode session export` output: {error}"))?;

        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            let detail = detail.trim();

            return Err(if detail.is_empty() {
                format!("could not export OpenCode session `{session}`")
            } else {
                format!("could not export OpenCode session `{session}`: {detail}")
            });
        }

        fs::read(&path)
            .map_err(|error| format!("could not read the OpenCode session export: {error}"))
    })();

    let _ = fs::remove_file(&path);

    result
}

/// A unique path for one export.
fn export_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    std::env::temp_dir().join(format!(
        "mm-opencode-export-{}-{nanos}-{unique}.json",
        std::process::id()
    ))
}

/// Parse the JSON produced by `opencode session export`.
pub fn parse(export: &[u8], session_id: &str) -> Result<ChatMessage, String> {
    parse_with_root(export, session_id, None)
}

/// Parse the JSON produced by `opencode session export`.
pub fn parse_with_root(
    export: &[u8],
    session_id: &str,
    repo_root: Option<&Path>,
) -> Result<ChatMessage, String> {
    let value: Value = serde_json::from_slice(export)
        .map_err(|error| format!("invalid JSON from `opencode session export`: {error}"))?;

    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let title = value
        .get("info")
        .and_then(|info| info.get("title"))
        .or_else(|| value.get("title"))
        .and_then(Value::as_str)
        .map(single_line)
        .filter(|title| !title.is_empty() && !is_placeholder_title(title))
        .map(|t| sanitize_file_urls(&t));

    let mut latest_prompt = None;
    let mut last_message = None;

    for message in messages {
        let kind = message_kind(message);

        match kind {
            "user" => {
                if let Some(line) = user_text(message).as_deref().and_then(first_line) {
                    latest_prompt = Some(sanitize_file_urls(&line));
                }
            }
            "assistant" => {
                if let Some(text) = assistant_copy_text(message) {
                    last_message = Some(sanitize_file_urls(&text));
                }
            }
            "compaction" => {
                if let Some(text) = compaction_text(message) {
                    last_message = Some(sanitize_file_urls(&text));
                }
            }
            _ => {}
        }
    }

    let raw_body = last_message.ok_or_else(|| {
        format!(
            "OpenCode session `{session_id}` has no assistant text or completed compaction summary to copy"
        )
    })?;

    let sanitized_body = sanitize_chat_body(&raw_body, repo_root);

    let (subject, subject_source) = match title {
        Some(title) => {
            let sanitized = sanitize_current_path(&title, repo_root);
            (sanitized, SubjectSource::Title)
        }
        None => match latest_prompt {
            Some(prompt) => {
                let sanitized = sanitize_current_path(&prompt, repo_root);
                (
                    truncate(&sanitized, FALLBACK_SUBJECT_MAX),
                    SubjectSource::LatestPrompt,
                )
            }
            None => {
                let line = first_line(&sanitized_body).unwrap_or_else(|| sanitized_body.clone());
                let sanitized = sanitize_current_path(&line, repo_root);
                (
                    truncate(&sanitized, FALLBACK_SUBJECT_MAX),
                    SubjectSource::LastMessage,
                )
            }
        },
    };

    Ok(ChatMessage {
        session_id: session_id.to_string(),
        subject: sanitize_file_urls(&subject),
        body: sanitized_body,
        subject_source,
    })
}

/// The `type` or `role` field of an exported message.
fn message_kind(message: &Value) -> &str {
    message
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| message.get("role").and_then(Value::as_str))
        .unwrap_or("")
}

/// Text of a user message.
fn user_text(message: &Value) -> Option<String> {
    non_empty_text(message.get("text")).or_else(|| content_text(message))
}

/// Visible text of an assistant message, joined like OpenCode's `/copy`.
fn assistant_copy_text(message: &Value) -> Option<String> {
    copy_content_text(message).or_else(|| non_empty_text(message.get("text")))
}

/// The summary of a completed compaction message.
fn compaction_text(message: &Value) -> Option<String> {
    let completed = message.get("status").and_then(Value::as_str) == Some("completed");

    if completed {
        non_empty_text(message.get("summary"))
    } else {
        None
    }
}

/// Join the `text` parts of a `content` array the way `/copy` does.
fn copy_content_text(message: &Value) -> Option<String> {
    match message.get("content")? {
        Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Value::Array(parts) => {
            let texts: Vec<&str> = parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect();
            let text = texts.join("\n");
            let text = text.trim();

            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        }
        _ => None,
    }
}

/// Read a non-blank JSON string.
fn non_empty_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Join the `text` parts of a `content` array.
fn content_text(message: &Value) -> Option<String> {
    match message.get("content")? {
        Value::String(text) => Some(text.clone()).filter(|text| !text.trim().is_empty()),
        Value::Array(parts) => {
            let texts: Vec<String> = parts
                .iter()
                .filter_map(part_text)
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
                .collect();

            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n\n"))
            }
        }
        _ => None,
    }
}

/// Text of one content part, ignoring reasoning, tools and other metadata.
fn part_text(part: &Value) -> Option<String> {
    match part {
        Value::String(text) => Some(text.clone()),
        Value::Object(_) => {
            let kind = part.get("type").and_then(Value::as_str);

            if matches!(kind, Some("text") | None) {
                part.get("text").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Collapse a title onto one line.
fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// First non-empty line, trimmed.
fn first_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// OpenCode and exported trajectories create placeholder titles before the title agent runs.
fn is_placeholder_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    lower.starts_with("new session")
        || lower == "cascade trajectory"
        || lower == "trajectory"
        || lower == "devin trajectory"
        || lower == "devin session"
        || lower == "conversation"
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = r#"{
        "info": { "id": "ses_test", "title": "Add chat import" },
        "messages": [
            { "type": "user", "text": "Please add chat import.\nUse the title." },
            {
                "type": "assistant",
                "content": [
                    { "type": "reasoning", "text": "not visible" },
                    { "type": "text", "text": "Implemented the import." },
                    { "type": "tool", "name": "shell" }
                ]
            }
        ]
    }"#;

    #[test]
    fn uses_the_title_as_the_subject() {
        let message = parse(EXPORT.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.subject, "Add chat import");
        assert_eq!(message.subject_source, SubjectSource::Title);
        assert_eq!(message.session_id, "ses_test");
    }

    #[test]
    fn copies_only_visible_text_from_the_last_assistant_message() {
        let message = parse(EXPORT.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.body, "Implemented the import.");
        assert!(!message.body.contains("not visible"));
        assert!(!message.body.contains("shell"));
        assert!(!message.body.contains("Please add chat import."));
    }

    #[test]
    fn renders_a_subject_and_body() {
        let message = parse(EXPORT.as_bytes(), "ses_test").unwrap();

        assert_eq!(
            message.message(None),
            "Add chat import\n\nImplemented the import."
        );
    }

    #[test]
    fn an_override_replaces_the_subject_only() {
        let message = parse(EXPORT.as_bytes(), "ses_test").unwrap();

        assert_eq!(
            message.message(Some("feat: manual subject")),
            "feat: manual subject\n\nImplemented the import."
        );
    }

    #[test]
    fn copies_the_last_assistant_message_not_the_transcript() {
        let export = r#"{
            "info": { "title": "Explain the change" },
            "messages": [
                { "type": "user", "text": "first request" },
                { "type": "assistant", "content": [{ "type": "text", "text": "first answer" }] },
                { "type": "user", "text": "follow up" },
                { "type": "assistant", "content": [{ "type": "text", "text": "second answer" }] }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.body, "second answer");
    }

    #[test]
    fn joins_text_parts_with_a_newline_like_copy() {
        let export = r#"{
            "info": { "title": "Explain the change" },
            "messages": [
                {
                    "type": "assistant",
                    "content": [
                        { "type": "text", "text": "first paragraph" },
                        { "type": "reasoning", "text": "hidden" },
                        { "type": "text", "text": "second paragraph" }
                    ]
                }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.body, "first paragraph\nsecond paragraph");
    }

    #[test]
    fn copies_a_completed_compaction_summary() {
        let export = r#"{
            "info": { "title": "Explain the change" },
            "messages": [
                { "type": "assistant", "content": [{ "type": "text", "text": "earlier answer" }] },
                { "type": "compaction", "status": "completed", "summary": "  Summary of the session.  " }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.body, "Summary of the session.");
    }

    #[test]
    fn ignores_an_incomplete_compaction() {
        let export = r#"{
            "info": { "title": "Explain the change" },
            "messages": [
                { "type": "assistant", "content": [{ "type": "text", "text": "earlier answer" }] },
                { "type": "compaction", "status": "running", "summary": "not ready" }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.body, "earlier answer");
    }

    #[test]
    fn falls_back_to_the_latest_prompt_without_a_title() {
        let export = r#"{
            "info": { "id": "ses_test" },
            "messages": [
                { "type": "user", "text": "first request" },
                { "type": "assistant", "content": [{ "type": "text", "text": "first answer" }] },
                { "type": "user", "text": "\nlatest request\nwith more detail" }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.subject, "latest request");
        assert_eq!(message.subject_source, SubjectSource::LatestPrompt);
        assert_eq!(message.body, "first answer");
    }

    #[test]
    fn falls_back_to_the_last_message_without_a_title_or_prompt() {
        let export = r#"{
            "info": { "id": "ses_test" },
            "messages": [
                { "type": "assistant", "content": [{ "type": "text", "text": "answer only\nmore detail" }] }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.subject, "answer only");
        assert_eq!(message.subject_source, SubjectSource::LastMessage);
        assert_eq!(message.body, "answer only\nmore detail");
    }

    #[test]
    fn ignores_placeholder_titles() {
        let export = r#"{
            "info": { "title": "New session - 2026-10-24T09:16:27.949Z" },
            "messages": [
                { "type": "user", "text": "real request" },
                { "type": "assistant", "content": [{ "type": "text", "text": "the answer" }] }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.subject, "real request");
        assert_eq!(message.subject_source, SubjectSource::LatestPrompt);
    }

    #[test]
    fn truncates_a_long_fallback_subject() {
        let export = r#"{
            "info": {},
            "messages": [
                { "type": "user", "text": "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz" },
                { "type": "assistant", "content": [{ "type": "text", "text": "the answer" }] }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.subject.chars().count(), FALLBACK_SUBJECT_MAX);
        assert!(message.subject.ends_with("..."));
    }

    #[test]
    fn rejects_an_export_without_copyable_text() {
        let export = r#"{ "info": { "title": "Empty" }, "messages": [] }"#;
        let error = parse(export.as_bytes(), "ses_test").unwrap_err();

        assert!(error.contains("no assistant text"));
        assert!(error.contains("ses_test"));
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse(b"{", "ses_test").is_err());
    }

    #[test]
    fn parses_antigravity_transcript() {
        let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nadd support for antigravity logs\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nsome metadata\n</ADDITIONAL_METADATA>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"ls"}]}
{"step_index":2,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Implemented support for Antigravity logs.\n\n- Added parser\n- Added tests"}
"#;

        let message = parse_antigravity(transcript.as_bytes(), "conv_123").unwrap();

        assert_eq!(message.session_id, "conv_123");
        assert_eq!(message.subject, "add support for antigravity logs");
        assert_eq!(message.subject_source, SubjectSource::LatestPrompt);
        assert_eq!(
            message.body,
            "Implemented support for Antigravity logs.\n\n- Added parser\n- Added tests"
        );
        assert_eq!(
            message.message(None),
            "add support for antigravity logs\n\nImplemented support for Antigravity logs.\n\n- Added parser\n- Added tests"
        );
    }

    #[test]
    fn antigravity_cleans_file_mentions_in_prompt() {
        let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\n@[/path/to/file.rs:L1-L10] fix null check\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Fixed the null check."}
"#;

        let message = parse_antigravity(transcript.as_bytes(), "conv_456").unwrap();
        assert_eq!(message.subject, "fix null check");
        assert_eq!(message.body, "Fixed the null check.");
    }

    #[test]
    fn antigravity_falls_back_to_first_line_of_assistant_message() {
        let transcript = r#"{"step_index":0,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"First line of response\nSecond line of response"}
"#;

        let message = parse_antigravity(transcript.as_bytes(), "conv_789").unwrap();
        assert_eq!(message.subject, "First line of response");
        assert_eq!(message.subject_source, SubjectSource::LastMessage);
        assert_eq!(
            message.body,
            "First line of response\nSecond line of response"
        );
    }

    #[test]
    fn antigravity_rejects_transcript_without_assistant_message() {
        let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nonly request\n</USER_REQUEST>"}
"#;

        let error = parse_antigravity(transcript.as_bytes(), "conv_empty").unwrap_err();
        assert!(error.contains("no assistant response text"));
        assert!(error.contains("conv_empty"));
    }

    #[test]
    fn sanitize_file_urls_replaces_macos_user_paths() {
        assert_eq!(sanitize_file_urls("file:///Users/user/..."), "~/...");
        assert_eq!(
            sanitize_file_urls("Check file:///Users/a0/project/file.rs"),
            "Check ~/project/file.rs"
        );
        assert_eq!(
            sanitize_file_urls("[Cargo.toml](file:///Users/john.doe/w/repo/Cargo.toml)"),
            "[Cargo.toml](~/w/repo/Cargo.toml)"
        );
        assert_eq!(
            sanitize_file_urls("directory file:///Users/alice/"),
            "directory ~/"
        );
        assert_eq!(sanitize_file_urls("look in file:///Users/bob"), "look in ~");
        assert_eq!(
            sanitize_file_urls("link at (file:///Users/carol/foo.rs#L10-L20)"),
            "link at (~/foo.rs#L10-L20)"
        );
    }

    #[test]
    fn sanitize_file_urls_replaces_linux_and_windows_paths() {
        assert_eq!(
            sanitize_file_urls("file:///home/user/code/main.rs"),
            "~/code/main.rs"
        );
        assert_eq!(
            sanitize_file_urls("file:///C:/Users/user/project/file.rs"),
            "~/project/file.rs"
        );
        assert_eq!(
            sanitize_file_urls("file://localhost/Users/user/project/file.rs"),
            "~/project/file.rs"
        );
    }

    #[test]
    fn sanitize_file_urls_replaces_multiple_occurrences() {
        let input =
            "Modified [a](file:///Users/alice/repo/a.rs) and [b](file:///Users/bob/repo/b.rs)";
        assert_eq!(
            sanitize_file_urls(input),
            "Modified [a](~/repo/a.rs) and [b](~/repo/b.rs)"
        );
    }

    #[test]
    fn sanitize_file_urls_leaves_non_user_urls_intact() {
        assert_eq!(sanitize_file_urls("file:///etc/hosts"), "file:///etc/hosts");
        assert_eq!(
            sanitize_file_urls("file:///Users//empty"),
            "file:///Users//empty"
        );
        assert_eq!(
            sanitize_file_urls("https://example.com/Users/user"),
            "https://example.com/Users/user"
        );
    }

    #[test]
    fn antigravity_transcript_sanitizes_file_urls() {
        let transcript = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nfix file:///Users/secret_user/project/lib.rs\n</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Fixed bug in [lib.rs](file:///Users/secret_user/project/lib.rs)."}
"#;

        let message = parse_antigravity(transcript.as_bytes(), "conv_leak").unwrap();
        assert_eq!(message.subject, "fix ~/project/lib.rs");
        assert_eq!(message.body, "Fixed bug in [lib.rs](~/project/lib.rs).");
        assert!(!message.message(None).contains("secret_user"));
    }

    #[test]
    fn opencode_export_sanitizes_file_urls() {
        let export = r#"{
            "info": { "title": "Inspect file:///Users/opencode_user/repo/README.md" },
            "messages": [
                {
                    "type": "assistant",
                    "content": [{ "type": "text", "text": "See [doc](file:///Users/opencode_user/repo/README.md)." }]
                }
            ]
        }"#;

        let message = parse(export.as_bytes(), "ses_leak").unwrap();
        assert_eq!(message.subject, "Inspect ~/repo/README.md");
        assert_eq!(message.body, "See [doc](~/repo/README.md).");
        assert!(!message.message(None).contains("opencode_user"));
    }

    #[test]
    fn parses_claude_transcript() {
        let transcript = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Implement Claude Code support\nMore details"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"planning"},{"type":"text","text":"Implemented Claude Code support."}]}}
"#;

        let message = parse_claude(transcript.as_bytes(), "sess_claude_1").unwrap();
        assert_eq!(message.session_id, "sess_claude_1");
        assert_eq!(message.subject, "Implement Claude Code support");
        assert_eq!(message.subject_source, SubjectSource::LatestPrompt);
        assert_eq!(message.body, "Implemented Claude Code support.");
        assert_eq!(
            message.message(None),
            "Implement Claude Code support\n\nImplemented Claude Code support."
        );
    }

    #[test]
    fn claude_ignores_tool_results_for_prompt() {
        let transcript = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"original user prompt"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"bash","input":{"command":"cargo test"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"test output"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"All tests passed."}]}}
"#;

        let message = parse_claude(transcript.as_bytes(), "sess_claude_2").unwrap();
        assert_eq!(message.subject, "original user prompt");
        assert_eq!(message.body, "All tests passed.");
    }

    #[test]
    fn claude_falls_back_to_assistant_first_line() {
        let transcript = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"First line of claude answer\nSecond line"}]}}
"#;

        let message = parse_claude(transcript.as_bytes(), "sess_claude_3").unwrap();
        assert_eq!(message.subject, "First line of claude answer");
        assert_eq!(message.subject_source, SubjectSource::LastMessage);
        assert_eq!(message.body, "First line of claude answer\nSecond line");
    }

    #[test]
    fn claude_sanitizes_file_urls() {
        let transcript = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Check file:///Users/myuser/project/main.rs"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Modified file:///Users/myuser/project/main.rs."}]}}
"#;

        let message = parse_claude(transcript.as_bytes(), "sess_claude_4").unwrap();
        assert_eq!(message.subject, "Check ~/project/main.rs");
        assert_eq!(message.body, "Modified ~/project/main.rs.");
        assert!(!message.message(None).contains("myuser"));
    }

    #[test]
    fn claude_rejects_empty_assistant() {
        let transcript = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"only prompt"}]}}
"#;

        let err = parse_claude(transcript.as_bytes(), "sess_claude_5").unwrap_err();
        assert!(err.contains("no assistant response text"));
        assert!(err.contains("sess_claude_5"));
    }

    #[test]
    fn encodes_claude_project_path() {
        assert_eq!(
            encode_claude_project_path(Path::new("/Users/a0/w/auto-commit/rust")),
            "-Users-a0-w-auto-commit-rust"
        );
        assert_eq!(
            encode_claude_project_path(Path::new("C:\\Users\\user\\project")),
            "C--Users-user-project"
        );
    }

    #[test]
    fn parses_codex_transcript() {
        let transcript = r#"{"type":"session_meta","payload":{"id":"01a10459-bbd6-7f61-97ea-12a1ac1a7e91","cwd":"/Users/user/project"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Implement Codex log support\nMore details"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Implemented Codex log support."}]}}
"#;

        let message = parse_codex(transcript.as_bytes(), "codex_1", None).unwrap();
        assert_eq!(message.session_id, "codex_1");
        assert_eq!(message.subject, "Implement Codex log support");
        assert_eq!(message.subject_source, SubjectSource::LatestPrompt);
        assert_eq!(message.body, "Implemented Codex log support.");
        assert_eq!(
            message.message(None),
            "Implement Codex log support\n\nImplemented Codex log support."
        );
    }

    #[test]
    fn parses_codex_with_thread_name_title() {
        let transcript = r#"{"type":"session_meta","payload":{"id":"01a10459-bbd6-7f61-97ea-12a1ac1a7e91","cwd":"/Users/user/project"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"User request line\nDetails"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}
"#;

        let message = parse_codex(
            transcript.as_bytes(),
            "01a10459-bbd6-7f61-97ea-12a1ac1a7e91",
            Some("Create repository contributor guide"),
        )
        .unwrap();
        assert_eq!(message.subject, "Create repository contributor guide");
        assert_eq!(message.subject_source, SubjectSource::Title);
        assert_eq!(message.body, "Done.");
    }

    #[test]
    fn parses_codex_with_task_complete() {
        let transcript = r#"{"type":"session_meta","payload":{"id":"01a10459-bbd6-7f61-97ea-12a1ac1a7e91","cwd":"/Users/user/project"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Do some task"}]}}
{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Completed the task successfully."}}
"#;

        let message = parse_codex(transcript.as_bytes(), "codex_tc", None).unwrap();
        assert_eq!(message.subject, "Do some task");
        assert_eq!(message.body, "Completed the task successfully.");
    }

    #[test]
    fn codex_falls_back_to_assistant_first_line() {
        let transcript = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"First line of codex answer\nSecond line"}]}}
"#;

        let message = parse_codex(transcript.as_bytes(), "codex_fallback", None).unwrap();
        assert_eq!(message.subject, "First line of codex answer");
        assert_eq!(message.subject_source, SubjectSource::LastMessage);
        assert_eq!(message.body, "First line of codex answer\nSecond line");
    }

    #[test]
    fn codex_sanitizes_file_urls() {
        let transcript = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Check file:///Users/myuser/project/main.rs"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Modified file:///Users/myuser/project/main.rs."}]}}
"#;

        let message = parse_codex(transcript.as_bytes(), "codex_leak", None).unwrap();
        assert_eq!(message.subject, "Check ~/project/main.rs");
        assert_eq!(message.body, "Modified ~/project/main.rs.");
        assert!(!message.message(None).contains("myuser"));
    }

    #[test]
    fn codex_rejects_empty_assistant() {
        let transcript = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"only prompt"}]}}
"#;

        let err = parse_codex(transcript.as_bytes(), "codex_empty", None).unwrap_err();
        assert!(err.contains("no assistant response text"));
        assert!(err.contains("codex_empty"));
    }

    #[test]
    fn sanitize_current_path_replaces_repo_path_with_dot_slash() {
        let home = Path::new("/Users/a0");
        let repo = Path::new("/Users/a0/w/auto-commit/rust");

        // Trailing slash in tilde form
        assert_eq!(
            sanitize_current_path_with_home(
                "Updated [chat.rs](~/w/auto-commit/rust/src/chat.rs)",
                Some(repo),
                Some(home)
            ),
            "Updated [chat.rs](./src/chat.rs)"
        );

        // Raw directory with trailing slash
        assert_eq!(
            sanitize_current_path_with_home("In ~/w/auto-commit/rust/", Some(repo), Some(home)),
            "In ./"
        );

        // Delimiter boundary without trailing slash
        assert_eq!(
            sanitize_current_path_with_home(
                "In directory [rust](~/w/auto-commit/rust)",
                Some(repo),
                Some(home)
            ),
            "In directory [rust](./)"
        );

        // Absolute path with trailing slash
        assert_eq!(
            sanitize_current_path_with_home(
                "Inspect /Users/a0/w/auto-commit/rust/src/main.rs",
                Some(repo),
                Some(home)
            ),
            "Inspect ./src/main.rs"
        );

        // file:// URL
        assert_eq!(
            sanitize_current_path_with_home(
                "Look at file:///Users/a0/w/auto-commit/rust/src/chat.rs",
                Some(repo),
                Some(home)
            ),
            "Look at ./src/chat.rs"
        );

        // file://localhost URL
        assert_eq!(
            sanitize_current_path_with_home(
                "Look at file://localhost/Users/a0/w/auto-commit/rust/README.md",
                Some(repo),
                Some(home)
            ),
            "Look at ./README.md"
        );

        // Quoted path like in user requests
        assert_eq!(
            sanitize_current_path_with_home(
                "if current path like \"~/w/auto-commit/rust/\" is current path",
                Some(repo),
                Some(home)
            ),
            "if current path like \"./\" is current path"
        );

        // Raw directory without trailing slash
        assert_eq!(
            sanitize_current_path_with_home("In ~/w/auto-commit/rust", Some(repo), Some(home)),
            "In ./"
        );

        // Colon delimiter
        assert_eq!(
            sanitize_current_path_with_home(
                "At ~/w/auto-commit/rust: line 42",
                Some(repo),
                Some(home)
            ),
            "At ./: line 42"
        );

        // Markdown bold
        assert_eq!(
            sanitize_current_path_with_home(
                "In **~/w/auto-commit/rust** directory",
                Some(repo),
                Some(home)
            ),
            "In **./** directory"
        );

        // file:// with tilde
        assert_eq!(
            sanitize_current_path_with_home(
                "Look at file://~/w/auto-commit/rust/src/main.rs",
                Some(repo),
                Some(home)
            ),
            "Look at ./src/main.rs"
        );
        assert_eq!(
            sanitize_current_path_with_home(
                "Look at file:///~/w/auto-commit/rust/src/main.rs",
                Some(repo),
                Some(home)
            ),
            "Look at ./src/main.rs"
        );
    }

    #[test]
    fn sanitize_current_path_preserves_other_paths() {
        let home = Path::new("/Users/a0");
        let repo = Path::new("/Users/a0/w/auto-commit/rust");

        // Other project in same home
        assert_eq!(
            sanitize_current_path_with_home(
                "See ~/other-project/src/main.rs",
                Some(repo),
                Some(home)
            ),
            "See ~/other-project/src/main.rs"
        );

        // Similar prefix directory (not delimited)
        assert_eq!(
            sanitize_current_path_with_home(
                "See ~/w/auto-commit/rust-extra/src/main.rs",
                Some(repo),
                Some(home)
            ),
            "See ~/w/auto-commit/rust-extra/src/main.rs"
        );
    }

    #[test]
    fn parses_devin_atif_json() {
        let transcript = r#"{
            "session_id": "devin-session-123",
            "schema_version": "ATIF-v1.7",
            "steps": [
                {
                    "node_id": "step_1",
                    "role": "user",
                    "content": {
                        "text": "Add Cascade and Devin transcript parsing support"
                    }
                },
                {
                    "node_id": "step_2",
                    "role": "assistant",
                    "chat_message": {
                        "content": "Added support for reading Devin and Cascade transcripts."
                    }
                }
            ]
        }"#;

        let msg = parse_devin(transcript.as_bytes(), "devin-session-123").unwrap();
        assert_eq!(
            msg.subject,
            "Add Cascade and Devin transcript parsing support"
        );
        assert_eq!(
            msg.body,
            "Added support for reading Devin and Cascade transcripts."
        );
        assert_eq!(msg.session_id, "devin-session-123");
        assert_eq!(msg.subject_source, SubjectSource::LatestPrompt);
    }

    #[test]
    fn parses_devin_jsonl() {
        let transcript = r#"{"role":"user","content":"Fix database connection retry"}
{"role":"assistant","content":"Implemented exponential backoff on connection retries."}
"#;

        let msg = parse_devin(transcript.as_bytes(), "devin-jsonl-1").unwrap();
        assert_eq!(msg.subject, "Fix database connection retry");
        assert_eq!(
            msg.body,
            "Implemented exponential backoff on connection retries."
        );
        assert_eq!(msg.subject_source, SubjectSource::LatestPrompt);
    }

    #[test]
    fn parses_devin_with_title() {
        let transcript = r#"{
            "session_id": "devin-title-test",
            "title": "feat(api): add auth endpoints",
            "steps": [
                {"role": "user", "content": "setup auth"},
                {"role": "assistant", "content": "Auth endpoints created."}
            ]
        }"#;

        let msg = parse_devin(transcript.as_bytes(), "devin-title-test").unwrap();
        assert_eq!(msg.subject, "feat(api): add auth endpoints");
        assert_eq!(msg.body, "Auth endpoints created.");
        assert_eq!(msg.subject_source, SubjectSource::Title);
    }

    #[test]
    fn parses_cascade_markdown_trajectory() {
        let transcript = r#"# Cascade Trajectory: Refactor router

## User
Refactor router to use async handlers

## Cascade
Router has been refactored to use async handlers across all routes.
"#;

        let msg = parse_devin(transcript.as_bytes(), "cascade-md").unwrap();
        assert_eq!(msg.subject, "Refactor router");
        assert_eq!(
            msg.body,
            "Router has been refactored to use async handlers across all routes."
        );
        assert_eq!(msg.subject_source, SubjectSource::Title);
    }

    #[test]
    fn devin_rejects_empty_assistant() {
        let transcript = r#"{"steps":[{"role":"user","content":"only a prompt"}]}"#;
        let err = parse_devin(transcript.as_bytes(), "empty-devin").unwrap_err();
        assert!(err.contains("no assistant response text"));
        assert!(err.contains("empty-devin"));
    }

    #[test]
    fn parses_freebuff_chat_messages_json() {
        let transcript = r#"[
            {
                "id": "msg-1",
                "variant": "user",
                "content": "Add Freebuff support to mm",
                "timestamp": "2026-10-29T10:00:00.000Z"
            },
            {
                "id": "msg-2",
                "variant": "ai",
                "content": "raw content",
                "blocks": [
                    {
                        "type": "text",
                        "content": "Thinking about the implementation...",
                        "textType": "reasoning"
                    },
                    {
                        "type": "tool",
                        "toolCallId": "call-1",
                        "toolName": "read_file",
                        "input": {}
                    },
                    {
                        "type": "text",
                        "content": "Added Freebuff and Codebuff support to mm."
                    },
                    {
                        "type": "agent",
                        "blocks": [
                            {
                                "type": "text",
                                "content": "Updated CLI flags and documentation."
                            }
                        ]
                    }
                ],
                "timestamp": "2026-10-29T10:01:00.000Z"
            }
        ]"#;

        let msg = parse_freebuff(transcript.as_bytes(), "freebuff-session-123").unwrap();
        assert_eq!(msg.subject, "Add Freebuff support to mm");
        assert_eq!(
            msg.body,
            "Added Freebuff and Codebuff support to mm.\n\nUpdated CLI flags and documentation."
        );
        assert_eq!(msg.session_id, "freebuff-session-123");
        assert_eq!(msg.subject_source, SubjectSource::LatestPrompt);
    }

    #[test]
    fn parses_freebuff_fallback_content() {
        let transcript = r#"[
            {"variant": "user", "content": "Refactor error handling"},
            {"variant": "ai", "content": "<think>need to check err</think>Refactored error handling across modules."}
        ]"#;

        let msg = parse_freebuff(transcript.as_bytes(), "fb-fallback").unwrap();
        assert_eq!(msg.subject, "Refactor error handling");
        assert_eq!(msg.body, "Refactored error handling across modules.");
        assert_eq!(msg.subject_source, SubjectSource::LatestPrompt);
    }

    #[test]
    fn parses_freebuff_with_title() {
        let transcript = r#"{
            "title": "feat(chat): support freebuff",
            "messages": [
                {"variant": "user", "content": "hello freebuff"},
                {"variant": "ai", "content": "Implemented freebuff integration."}
            ]
        }"#;

        let msg = parse_freebuff(transcript.as_bytes(), "fb-title").unwrap();
        assert_eq!(msg.subject, "feat(chat): support freebuff");
        assert_eq!(msg.body, "Implemented freebuff integration.");
        assert_eq!(msg.subject_source, SubjectSource::Title);
    }

    #[test]
    fn parses_freebuff_jsonl() {
        let transcript = r#"{"variant":"user","content":"Fix git diff detection"}
{"variant":"assistant","content":"Fixed edge case in git diff status parser."}
"#;

        let msg = parse_freebuff(transcript.as_bytes(), "fb-jsonl").unwrap();
        assert_eq!(msg.subject, "Fix git diff detection");
        assert_eq!(msg.body, "Fixed edge case in git diff status parser.");
        assert_eq!(msg.subject_source, SubjectSource::LatestPrompt);
    }

    #[test]
    fn freebuff_rejects_empty_assistant() {
        let transcript = r#"[{"variant":"user","content":"only a prompt"}]"#;
        let err = parse_freebuff(transcript.as_bytes(), "empty-fb").unwrap_err();
        assert!(err.contains("no assistant response text"));
        assert!(err.contains("empty-fb"));
    }

    #[test]
    fn resolves_freebuff_session_and_latest_log() {
        let tmp = std::env::temp_dir().join(format!("mm-test-fb-res-{}", std::process::id()));
        let project_dir = tmp
            .join("projects")
            .join("my-repo")
            .join("chats")
            .join("chat-abc");
        fs::create_dir_all(&project_dir).unwrap();

        let transcript = r#"[
            {"variant":"user","content":"Support Freebuff in mm"},
            {"variant":"ai","content":"Successfully added Freebuff support."}
        ]"#;
        fs::write(project_dir.join("chat-messages.json"), transcript).unwrap();

        // 1. Resolve by direct path to file
        let (id, p) = resolve_freebuff_session(
            &project_dir.join("chat-messages.json").to_string_lossy(),
            None,
        )
        .unwrap();
        assert_eq!(id, "chat-abc");
        assert!(p.is_file());

        // 2. Resolve by direct path to directory
        let (id, p) = resolve_freebuff_session(&project_dir.to_string_lossy(), None).unwrap();
        assert_eq!(id, "chat-abc");
        assert!(p.is_file());

        // 3. Resolve inside config dir
        let repo_path = PathBuf::from("/some/path/to/my-repo");
        let (id, p) = resolve_freebuff_session_in(&tmp, "chat-abc", Some(&repo_path)).unwrap();
        assert_eq!(id, "chat-abc");
        assert!(p.is_file());

        // 4. Find latest log inside config dir
        let (id, p) = find_latest_freebuff_log_in(&tmp, Some(&repo_path)).unwrap();
        assert_eq!(id, "chat-abc");
        assert!(p.is_file());

        let _ = fs::remove_dir_all(tmp);
    }

    #[test]
    fn strips_ansi_escapes_from_text() {
        assert_eq!(strip_ansi_escapes("plain text"), "plain text");
        assert_eq!(
            strip_ansi_escapes("\x1b[32mSuccess!\x1b[0m \x1b[1;31mError\x1b[0m"),
            "Success! Error"
        );
        assert_eq!(
            strip_ansi_escapes("\x1b]0;terminal title\x07Hello world"),
            "Hello world"
        );
        assert_eq!(
            strip_ansi_escapes("\x1b]0;terminal title\x1b\\Hello world"),
            "Hello world"
        );
        assert_eq!(
            strip_ansi_escapes("\x1b[2K\rDone!\r\nNext line"),
            "Done!\nNext line"
        );
    }

    #[test]
    fn strips_tool_residue_xml_blocks() {
        let input = "Start\n<tool_call>\n{\"name\": \"run_cmd\"}\n</tool_call>\nMiddle\n<thought>thinking...</thought>\nEnd";
        let cleaned = strip_tool_residue(input);
        assert_eq!(cleaned, "Start\n\nMiddle\n\nEnd");
    }

    #[test]
    fn strips_tool_residue_code_fences_and_cli_markers() {
        let input = "Fixed the bug.\n\n```tool_call\n{\"tool\": \"shell\"}\n```\n\n[Tool Output: tests passed]\n\nAll tests pass.";
        let cleaned = strip_tool_residue(input);
        assert_eq!(cleaned, "Fixed the bug.\n\nAll tests pass.");
    }

    #[test]
    fn strips_conversational_filler_greetings_and_leadins() {
        let input = "Sure! I will help you with that.\n\nHere is a summary:\n- Added feature A\n- Fixed bug B\n\nHope this helps! Let me know if you need anything else.";
        let cleaned = strip_conversational_filler(input);
        assert_eq!(cleaned, "- Added feature A\n- Fixed bug B");
    }

    #[test]
    fn strips_conversational_prefix_from_single_sentence() {
        let input = "Sure! I've updated the implementation to support custom scopes.";
        let cleaned = strip_conversational_filler(input);
        assert_eq!(
            cleaned,
            "I've updated the implementation to support custom scopes."
        );
    }

    #[test]
    fn preserves_structural_headings_and_lists() {
        let input = "### Summary of Changes\n\n1. Added unit test\n2. Updated documentation";
        let cleaned = strip_conversational_filler(input);
        assert_eq!(
            cleaned,
            "### Summary of Changes\n\n1. Added unit test\n2. Updated documentation"
        );
    }

    #[test]
    fn full_sanitize_chat_body_integration() {
        let input = "\x1b[32mSure! I will help you with this task.\x1b[0m\n\n<tool_call>\n{\"name\": \"test\"}\n</tool_call>\n\nHere is a summary of the changes:\n- Fixed edge cases in parser\n- Added test suite\n\nLet me know if you have any questions!";
        let cleaned = sanitize_chat_body(input, None);
        assert_eq!(cleaned, "- Fixed edge cases in parser\n- Added test suite");
    }
}
