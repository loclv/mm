//! Copy a commit message from an OpenCode chat session.
//!
//! OpenCode owns its session database, but it exposes a stable JSON export
//! through `opencode session export <id>`. The export contains the session
//! title and the visible transcript. `mm --chat` uses the title as the commit
//! subject and the user prompts plus assistant text as the body. Tool calls,
//! reasoning, system messages and compaction summaries are not copied.

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
}

/// A commit message copied from an OpenCode session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Session the message came from.
    pub session_id: String,
    /// Subject line.
    pub subject: String,
    /// Visible user and assistant text, in order.
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

    let mut blocks = Vec::new();
    let mut latest_prompt = None;

    for message in messages {
        let kind = message_kind(message);

        let text = match kind {
            "user" => {
                let text = user_text(message);

                if let Some(line) = text.as_deref().and_then(first_line) {
                    latest_prompt = Some(line);
                }

                text
            }
            "assistant" => assistant_text(message),
            _ => None,
        };

        let Some(text) = text.map(|text| text.trim().to_string()) else {
            continue;
        };

        if text.is_empty() {
            continue;
        }

        let label = match kind {
            "user" => "User",
            "assistant" => "Assistant",
            _ => continue,
        };

        blocks.push(format!("{label}:\n{text}"));
    }

    let body = blocks.join("\n\n");
    let (subject, subject_source) = match title {
        Some(title) => (title, SubjectSource::Title),
        None => {
            let prompt = latest_prompt.ok_or_else(|| {
                format!("OpenCode session `{session_id}` has no title and no user messages to copy")
            })?;

            (
                truncate(&prompt, FALLBACK_SUBJECT_MAX),
                SubjectSource::LatestPrompt,
            )
        }
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

/// Visible text of an assistant message, excluding reasoning and tools.
fn assistant_text(message: &Value) -> Option<String> {
    content_text(message).or_else(|| non_empty_text(message.get("text")))
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
    fn copies_only_visible_text_into_the_body() {
        let message = parse(EXPORT.as_bytes(), "ses_test").unwrap();

        assert_eq!(
            message.body,
            "User:\nPlease add chat import.\nUse the title.\n\nAssistant:\nImplemented the import."
        );
        assert!(!message.body.contains("not visible"));
        assert!(!message.body.contains("shell"));
    }

    #[test]
    fn renders_a_subject_and_body() {
        let message = parse(EXPORT.as_bytes(), "ses_test").unwrap();

        assert_eq!(
            message.message(None),
            "Add chat import\n\nUser:\nPlease add chat import.\nUse the title.\n\nAssistant:\nImplemented the import."
        );
    }

    #[test]
    fn an_override_replaces_the_subject_only() {
        let message = parse(EXPORT.as_bytes(), "ses_test").unwrap();

        assert_eq!(
            message.message(Some("feat: manual subject")),
            "feat: manual subject\n\nUser:\nPlease add chat import.\nUse the title.\n\nAssistant:\nImplemented the import."
        );
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
        assert!(message.body.contains("first request"));
        assert!(message.body.contains("latest request"));
    }

    #[test]
    fn ignores_placeholder_titles() {
        let export = r#"{
            "info": { "title": "New session - 2026-10-24T09:16:27.949Z" },
            "messages": [{ "type": "user", "text": "real request" }]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.subject, "real request");
        assert_eq!(message.subject_source, SubjectSource::LatestPrompt);
    }

    #[test]
    fn truncates_a_long_fallback_subject() {
        let export = r#"{
            "info": {},
            "messages": [{ "type": "user", "text": "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz" }]
        }"#;

        let message = parse(export.as_bytes(), "ses_test").unwrap();

        assert_eq!(message.subject.chars().count(), FALLBACK_SUBJECT_MAX);
        assert!(message.subject.ends_with("..."));
    }

    #[test]
    fn rejects_an_export_without_a_title_or_prompt() {
        let export = r#"{ "info": {}, "messages": [] }"#;
        let error = parse(export.as_bytes(), "ses_test").unwrap_err();

        assert!(error.contains("no title"));
        assert!(error.contains("ses_test"));
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse(b"{", "ses_test").is_err());
    }
}
