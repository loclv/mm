//! Copy a commit message from an OpenCode chat session.
//!
//! OpenCode owns its session database, but it exposes a stable JSON export
//! through `opencode session export <id>`. The export contains the session
//! title and the visible transcript. `mm --chat` uses the title as the commit
//! subject and copies the last assistant message as the body, matching
//! OpenCode's `/copy` command. A completed compaction summary is copied when it
//! is the last message. Tool calls, reasoning and system messages are skipped.

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
        let subject = subject_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.subject);

        if self.body.is_empty() {
            return subject.to_string();
        }

        format!("{subject}\n\n{}", self.body)
    }
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
                        latest_prompt = Some(cleaned);
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
                last_message = Some(text.to_string());
            }
        }
    }

    let body = last_message.ok_or_else(|| {
        format!(
            "Antigravity conversation `{conversation_id}` has no assistant response text to copy"
        )
    })?;

    let (subject, subject_source) = match latest_prompt {
        Some(prompt) => (
            truncate(&prompt, FALLBACK_SUBJECT_MAX),
            SubjectSource::LatestPrompt,
        ),
        None => {
            let line = first_line(&body).unwrap_or_else(|| body.clone());
            (
                truncate(&line, FALLBACK_SUBJECT_MAX),
                SubjectSource::LastMessage,
            )
        }
    };

    Ok(ChatMessage {
        session_id: conversation_id.to_string(),
        subject,
        body,
        subject_source,
    })
}

/// Parse an Antigravity transcript from a byte slice.
pub fn parse_antigravity(transcript: &[u8], conversation_id: &str) -> Result<ChatMessage, String> {
    parse_antigravity_reader(std::io::Cursor::new(transcript), conversation_id)
}

/// Load and parse an Antigravity transcript from disk.
pub fn load_antigravity(path: &Path, conversation_id: &str) -> Result<ChatMessage, String> {
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "could not open Antigravity transcript {}: {error}",
            path.display()
        )
    })?;
    let reader = std::io::BufReader::new(file);
    parse_antigravity_reader(reader, conversation_id)
}

/// Load and parse a session export or conversation log.
///
/// Sources supported:
/// - Explicit Antigravity conversation or OpenCode session via `session`.
/// - `OPENCODE_SESSION_ID` environment variable.
/// - `ANTIGRAVITY_CONVERSATION_ID` environment variable.
/// - Auto-detection of the latest Antigravity conversation log in `~/.gemini/antigravity-ide/brain`.
pub fn load(
    session: Option<&str>,
    repo_root: Option<&Path>,
    force_antigravity: bool,
    verbose: bool,
) -> Result<ChatMessage, String> {
    let trimmed_session = session.map(str::trim).filter(|value| !value.is_empty());

    if force_antigravity {
        let (id, path) = if let Some(sess) = trimmed_session {
            resolve_antigravity_session(sess).ok_or_else(|| {
                format!("could not find Antigravity conversation or transcript `{sess}`")
            })?
        } else if let Some((id, path)) = std::env::var(ANTIGRAVITY_SESSION_ENV)
            .or_else(|_| std::env::var("GEMINI_CONVERSATION_ID"))
            .ok()
            .and_then(|sess| {
                let s = sess.trim();
                if s.is_empty() {
                    None
                } else {
                    resolve_antigravity_session(s)
                }
            })
        {
            (id, path)
        } else {
            find_latest_antigravity_log(repo_root)
                .ok_or_else(|| "no Antigravity conversation log found".to_string())?
        };

        if verbose {
            eprintln!(
                "[mm] chat: using Antigravity log {} ({})",
                id,
                path.display()
            );
        }

        return load_antigravity(&path, &id);
    }

    if let Some(sess) = trimmed_session {
        if let Some((id, path)) = resolve_antigravity_session(sess) {
            if verbose {
                eprintln!(
                    "[mm] chat: using Antigravity log {} ({})",
                    id,
                    path.display()
                );
            }
            return load_antigravity(&path, &id);
        }

        if verbose {
            eprintln!("[mm] chat: exporting OpenCode session {sess}");
        }

        let export = export_session(sess)?;
        return parse(&export, sess);
    }

    if let Ok(opencode_session) = std::env::var(SESSION_ENV) {
        let sess = opencode_session.trim();
        if !sess.is_empty() {
            if verbose {
                eprintln!("[mm] chat: exporting OpenCode session {sess}");
            }

            let export = export_session(sess)?;
            return parse(&export, sess);
        }
        return Err(format!(
            "no OpenCode session selected; set {SESSION_ENV} or pass --chat-session <ID>"
        ));
    }

    if let Ok(agy_session) =
        std::env::var(ANTIGRAVITY_SESSION_ENV).or_else(|_| std::env::var("GEMINI_CONVERSATION_ID"))
    {
        let sess = agy_session.trim();
        let resolved = if !sess.is_empty() {
            resolve_antigravity_session(sess)
        } else {
            None
        };
        if let Some((id, path)) = resolved {
            if verbose {
                eprintln!(
                    "[mm] chat: using Antigravity log {} ({})",
                    id,
                    path.display()
                );
            }
            return load_antigravity(&path, &id);
        }
    }

    if let Some((id, path)) = find_latest_antigravity_log(repo_root) {
        if verbose {
            eprintln!(
                "[mm] chat: auto-detected Antigravity log {} ({})",
                id,
                path.display()
            );
        }
        return load_antigravity(&path, &id);
    }

    Err(format!(
        "no OpenCode session or Antigravity log found; set {SESSION_ENV}, {ANTIGRAVITY_SESSION_ENV}, or pass --chat-session <ID>"
    ))
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
        .filter(|title| !title.is_empty() && !is_placeholder_title(title));

    let mut latest_prompt = None;
    let mut last_message = None;

    for message in messages {
        let kind = message_kind(message);

        match kind {
            "user" => {
                if let Some(line) = user_text(message).as_deref().and_then(first_line) {
                    latest_prompt = Some(line);
                }
            }
            "assistant" => {
                if let Some(text) = assistant_copy_text(message) {
                    last_message = Some(text);
                }
            }
            "compaction" => {
                if let Some(text) = compaction_text(message) {
                    last_message = Some(text);
                }
            }
            _ => {}
        }
    }

    let body = last_message.ok_or_else(|| {
        format!(
            "OpenCode session `{session_id}` has no assistant text or completed compaction summary to copy"
        )
    })?;

    let (subject, subject_source) = match title {
        Some(title) => (title, SubjectSource::Title),
        None => match latest_prompt {
            Some(prompt) => (
                truncate(&prompt, FALLBACK_SUBJECT_MAX),
                SubjectSource::LatestPrompt,
            ),
            None => {
                let line = first_line(&body).unwrap_or_else(|| body.clone());

                (
                    truncate(&line, FALLBACK_SUBJECT_MAX),
                    SubjectSource::LastMessage,
                )
            }
        },
    };

    Ok(ChatMessage {
        session_id: session_id.to_string(),
        subject,
        body,
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

/// OpenCode creates placeholder titles before the title agent runs.
fn is_placeholder_title(title: &str) -> bool {
    title.to_lowercase().starts_with("new session")
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
}
