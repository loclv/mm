//! Copy a commit message from an OpenCode chat session.
//!
//! OpenCode owns its session database, but it exposes a stable JSON export
//! through `opencode session export <id>`. The export contains the session
//! title and the visible transcript. `mm --chat` uses the title as the commit
//! subject and copies the last assistant message as the body, matching
//! OpenCode's `/copy` command. A completed compaction summary is copied when it
//! is the last message. Tool calls, reasoning and system messages are skipped.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::policy::truncate;

/// Environment variable OpenCode sets for the current session.
pub const SESSION_ENV: &str = "OPENCODE_SESSION_ID";

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

/// A commit message copied from an OpenCode session.
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

/// Load and parse a session export.
///
/// The session is taken from `session`, then from [`SESSION_ENV`].
pub fn load(session: Option<&str>, verbose: bool) -> Result<ChatMessage, String> {
    let session = select_session(session)?;

    if verbose {
        eprintln!("[mm] chat: exporting OpenCode session {session}");
    }

    let export = export_session(&session)?;

    parse(&export, &session)
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

/// Choose an explicit session, or the one OpenCode is running.
fn select_session(session: Option<&str>) -> Result<String, String> {
    if let Some(session) = session.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(session.to_string());
    }

    if let Ok(session) = std::env::var(SESSION_ENV) {
        let session = session.trim();

        if !session.is_empty() {
            return Ok(session.to_string());
        }
    }

    Err(format!(
        "no OpenCode session selected; set {SESSION_ENV} or pass --chat-session <ID>"
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
}
