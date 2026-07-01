//! Parsing a Claude Code transcript (`.jsonl`) into lightweight events.
//!
//! The transcript schema drifts between Claude Code versions, so parsing is
//! deliberately tolerant: every line is read as a [`serde_json::Value`] and only
//! the handful of fields we care about are pulled out. Lines that fail to parse
//! are skipped rather than aborting the whole file.

use std::path::Path;

use jiff::Timestamp;
use serde_json::Value;
use tracing::debug;

use crate::error::{CoreError, Result};

/// One tool invocation extracted from an assistant turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    /// The tool name, e.g. `"Bash"`, `"Edit"`, `"Read"`.
    pub name: String,
    /// The primary file the tool touched, if the input carried one.
    pub file: Option<String>,
}

/// A concrete deliverable inferred from a shell command — the "what was done"
/// signal an executive cares about.
///
/// Only the classification (and, for merges, the PR number) is ever kept; the raw
/// command string is discarded so secrets in command lines never reach the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `gh pr create` — a pull request was opened.
    PrCreated,
    /// `gh pr merge` — a pull request was merged (number captured when present).
    PrMerged(Option<u32>),
    /// `git commit`.
    Commit,
    /// `git push`.
    Push,
    /// `gh release create` / `git tag` — a release or tag was cut.
    Release,
    /// A test runner was invoked (cargo/npm/pytest/go/…).
    Test,
    /// A build was invoked (cargo/npm/go/…).
    Build,
    /// `git revert` — a change was rolled back (a risk signal).
    Revert,
    /// `git push --force` / `-f` — history was rewritten (a risk signal).
    ForcePush,
}

/// A single transcript event, reduced to the fields the worklog needs.
#[derive(Debug, Clone)]
pub struct RawEvent {
    /// The event's unique id (`uuid`), used to deduplicate stored turns.
    pub uuid: Option<String>,
    /// The session this event belongs to (`sessionId`).
    pub session_id: Option<String>,
    /// The event timestamp, parsed from the RFC 3339 `timestamp` field.
    pub timestamp: Option<Timestamp>,
    /// The raw event `type` (`"user"`, `"assistant"`, `"system"`, …).
    pub event_type: String,
    /// The working directory recorded on the event, if any.
    pub cwd: Option<String>,
    /// The git branch recorded on the event, if any.
    pub git_branch: Option<String>,
    /// The conversation slug (auto-generated title), if any.
    pub slug: Option<String>,
    /// A genuine user prompt's text, if this event is one.
    pub user_text: Option<String>,
    /// Tool invocations carried by an assistant event.
    pub tool_uses: Vec<ToolUse>,
    /// Deliverables inferred from this event's shell commands.
    pub actions: Vec<Action>,
    /// Human-readable work items (commit subjects, PR titles) from this event.
    pub highlights: Vec<String>,
    /// Whether this is a user interrupt ("[Request interrupted by user]"),
    /// i.e. the user redirected the assistant mid-task — a friction signal.
    pub interrupted: bool,
}

/// Read and parse every event from a transcript file.
///
/// # Errors
/// Returns [`CoreError::Io`] if the file cannot be read. Individual malformed
/// lines are skipped, not propagated.
pub fn read_events(path: &Path) -> Result<Vec<RawEvent>> {
    let text = std::fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
    Ok(parse_events(&text))
}

/// Read events appended to a transcript since byte `offset`, for incremental
/// ingestion of a growing file.
///
/// Only whole lines are consumed: a partial final line (the file is still being
/// written) is left for next time. Returns the parsed events and the new offset
/// to persist. If `offset` is past the current end — e.g. the transcript was
/// truncated or rewritten by a compaction — reading restarts from the beginning.
///
/// # Errors
/// Returns [`CoreError::Io`] if the file cannot be opened, measured, or read.
pub fn read_events_from(path: &Path, offset: u64) -> Result<(Vec<RawEvent>, u64)> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    let mut file = std::fs::File::open(path).map_err(|e| CoreError::io(path, e))?;
    let len = file.metadata().map_err(|e| CoreError::io(path, e))?.len();
    let start = if offset > len { 0 } else { offset };
    file.seek(SeekFrom::Start(start))
        .map_err(|e| CoreError::io(path, e))?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| CoreError::io(path, e))?;
    // Consume only up to the last newline so a partial trailing line is retried.
    let consumed = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let text = String::from_utf8_lossy(&bytes[..consumed]);
    Ok((parse_events(&text), start + consumed as u64))
}

/// Parse events from the in-memory text of a transcript.
#[must_use]
pub fn parse_events(text: &str) -> Vec<RawEvent> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match serde_json::from_str::<Value>(line) {
            Ok(value) => parse_event(&value),
            Err(err) => {
                debug!(%err, "skipping unparsable transcript line");
                None
            },
        })
        .collect()
}

/// Convert one decoded JSON value into a [`RawEvent`], or `None` if it has no
/// `type` field (e.g. internal metadata lines).
fn parse_event(value: &Value) -> Option<RawEvent> {
    let event_type = value.get("type")?.as_str()?.to_owned();
    let message = value.get("message");

    let (user_text, tool_uses, actions, highlights) = match event_type.as_str() {
        "user" => (
            extract_user_text(message),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        "assistant" => (
            None,
            extract_tool_uses(message),
            extract_actions(message),
            extract_highlights(message),
        ),
        _ => (None, Vec::new(), Vec::new(), Vec::new()),
    };

    let interrupted = event_type == "user" && message_contains(message, "[Request interrupted");

    Some(RawEvent {
        uuid: str_field(value, "uuid"),
        session_id: str_field(value, "sessionId"),
        timestamp: value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp),
        event_type,
        cwd: str_field(value, "cwd"),
        git_branch: str_field(value, "gitBranch"),
        slug: str_field(value, "slug"),
        user_text,
        tool_uses,
        actions,
        highlights,
        interrupted,
    })
}

/// Whether a message's textual content contains `needle` (string or text parts).
fn message_contains(message: Option<&Value>, needle: &str) -> bool {
    let Some(content) = message.and_then(|m| m.get("content")) else {
        return false;
    };
    match content {
        Value::String(s) => s.contains(needle),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .any(|t| t.contains(needle)),
        _ => false,
    }
}

/// Parse an RFC 3339 timestamp, returning `None` (with a debug log) on failure.
fn parse_timestamp(raw: &str) -> Option<Timestamp> {
    match raw.parse::<Timestamp>() {
        Ok(ts) => Some(ts),
        Err(err) => {
            debug!(%raw, %err, "skipping unparsable timestamp");
            None
        },
    }
}

/// Pull a string field from an object, ignoring non-string values.
fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

/// Extract a genuine user prompt from a `user` event's message, filtering out
/// tool results and Claude Code's own command/meta injections.
fn extract_user_text(message: Option<&Value>) -> Option<String> {
    let content = message?.get("content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            // A tool result masquerades as a user event; it is not a prompt.
            if items.iter().any(is_tool_result) {
                return None;
            }
            let joined = items
                .iter()
                .filter_map(|item| match item.get("type").and_then(Value::as_str) {
                    Some("text") => item.get("text").and_then(Value::as_str),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                return None;
            }
            joined
        },
        _ => return None,
    };
    if is_real_prompt(&text) {
        Some(text)
    } else {
        None
    }
}

/// Whether a content array element is a tool result.
fn is_tool_result(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("tool_result")
}

/// Prefixes that mark Claude Code's own injected (non-human) text.
const SKIP_PREFIXES: [&str; 6] = [
    "<command-",
    "<local-command",
    "Caveat:",
    "<bash-stdout",
    "<bash-input",
    "<bash-stderr",
];

/// Heuristic: keep human prompts, drop Claude Code's injected text, control
/// markers, and pasted terminal output that isn't really a request.
fn is_real_prompt(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    if SKIP_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return false;
    }
    // Control markers Claude Code inserts, not user intent.
    if trimmed.contains("[Request interrupted") {
        return false;
    }
    // A pasted shell prompt (powerline glyph) is a stray paste, not a request.
    if trimmed.contains('❯') {
        return false;
    }
    true
}

/// Extract tool invocations from an `assistant` event's message content.
fn extract_tool_uses(message: Option<&Value>) -> Vec<ToolUse> {
    let Some(Value::Array(items)) = message.and_then(|m| m.get("content")) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|item| {
            let name = item.get("name").and_then(Value::as_str)?.to_owned();
            Some(ToolUse {
                name,
                file: tool_file(item.get("input")),
            })
        })
        .collect()
}

/// Pull the primary file path out of a tool's input, if it has one.
fn tool_file(input: Option<&Value>) -> Option<String> {
    let input = input?;
    for key in ["file_path", "notebook_path", "path"] {
        if let Some(p) = input.get(key).and_then(Value::as_str) {
            return Some(p.to_owned());
        }
    }
    None
}

/// Test-runner command fragments, by ecosystem.
const TEST_PATTERNS: [&str; 7] = [
    "cargo test",
    "cargo nextest",
    "just test",
    "npm test",
    "npm run test",
    "pytest",
    "go test",
];

/// Build command fragments, by ecosystem.
const BUILD_PATTERNS: [&str; 5] = [
    "cargo build",
    "just build",
    "npm run build",
    "go build",
    "dotnet build",
];

/// Infer deliverable [`Action`]s from an assistant event's `Bash` commands.
///
/// Only the classification is returned; the raw command text is never retained.
fn extract_actions(message: Option<&Value>) -> Vec<Action> {
    let Some(Value::Array(items)) = message.and_then(|m| m.get("content")) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter(|item| item.get("name").and_then(Value::as_str) == Some("Bash"))
        .filter_map(|item| item.get("input")?.get("command")?.as_str())
        .flat_map(classify_command)
        .collect()
}

/// Classify one shell command into the deliverables it performs.
fn classify_command(cmd: &str) -> Vec<Action> {
    let mut actions = Vec::new();
    // Each "gh pr merge" occurrence is one merge; grab a trailing PR number if any.
    for rest in cmd.split("gh pr merge").skip(1) {
        actions.push(Action::PrMerged(pr_number_after(rest)));
    }
    push_n(
        &mut actions,
        count_word(cmd, "gh pr create"),
        Action::PrCreated,
    );
    push_n(&mut actions, count_word(cmd, "git commit"), Action::Commit);
    push_n(&mut actions, count_word(cmd, "git push"), Action::Push);
    let releases = count_word(cmd, "gh release create") + count_word(cmd, "git tag");
    push_n(&mut actions, releases, Action::Release);
    push_n(&mut actions, count_any(cmd, &TEST_PATTERNS), Action::Test);
    push_n(&mut actions, count_any(cmd, &BUILD_PATTERNS), Action::Build);
    push_n(&mut actions, count_word(cmd, "git revert"), Action::Revert);
    let force = count_word(cmd, "push --force") + count_word(cmd, "push -f");
    push_n(&mut actions, force, Action::ForcePush);
    actions
}

/// Pull human-readable work items (commit subjects, PR titles) from an
/// assistant event's `Bash` commands.
fn extract_highlights(message: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = message.and_then(|m| m.get("content")) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter(|item| item.get("name").and_then(Value::as_str) == Some("Bash"))
        .filter_map(|item| item.get("input")?.get("command")?.as_str())
        .flat_map(command_highlights)
        .collect()
}

/// Commit subjects and PR titles named in one shell command.
fn command_highlights(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in cmd.split("commit -m").skip(1) {
        if let Some(subject) = first_arg(part) {
            out.push(subject);
        }
    }
    for part in cmd.split("--title").skip(1) {
        if let Some(title) = first_arg(part) {
            out.push(format!("PR: {title}"));
        }
    }
    out
}

/// Extract the first shell argument after a flag: a quoted string (single or
/// double) or, failing that, the rest of the line. Returns its first line,
/// truncated. Leading line-continuations and whitespace are skipped.
fn first_arg(after: &str) -> Option<String> {
    let s = after.trim_start_matches([' ', '\\', '\n', '\r', '\t', '=']);
    let mut chars = s.chars();
    let first = chars.next()?;
    let body = match first {
        '"' | '\'' => chars.take_while(|c| *c != first).collect::<String>(),
        // Unquoted: a bare token only, so we never swallow `&& next-command`.
        _ => s.split_whitespace().next().unwrap_or_default().to_owned(),
    };
    let line = first_line(&body, 100);
    // Drop shell noise (command substitution, heredocs, flags) — not a real subject.
    if line.is_empty() || line.starts_with(['$', '<', '>', '|', '&', '-', '`']) {
        None
    } else {
        Some(line)
    }
}

/// Push `n` copies of `action`.
fn push_n(actions: &mut Vec<Action>, n: usize, action: Action) {
    actions.extend(std::iter::repeat_n(action, n));
}

/// Total word-boundaried occurrences of any pattern in `cmd`.
fn count_any(cmd: &str, patterns: &[&str]) -> usize {
    patterns.iter().map(|p| count_word(cmd, p)).sum()
}

/// Count `pat` occurrences bounded by non-word characters on both sides, so
/// `cargo test` is not also counted as the substring `go test`.
fn count_word(cmd: &str, pat: &str) -> usize {
    let bytes = cmd.as_bytes();
    let mut count = 0;
    let mut from = 0;
    while let Some(pos) = cmd[from..].find(pat) {
        let abs = from + pos;
        let before_ok = abs == 0 || !is_word_byte(bytes[abs - 1]);
        let after = abs + pat.len();
        let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
        if before_ok && after_ok {
            count += 1;
        }
        from = abs + pat.len();
    }
    count
}

/// Whether a byte is part of a command "word" (alphanumeric, `-`, or `_`).
const fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// The first integer in the immediate arguments after `gh pr merge`, if present.
fn pr_number_after(rest: &str) -> Option<u32> {
    let segment = rest.split(['\n', ';', '&', '|']).next().unwrap_or(rest);
    segment
        .split_whitespace()
        .take(6)
        .find_map(|tok| tok.trim_start_matches('#').parse().ok())
}

/// Take the first non-empty line of `text`, truncated to `max` characters.
#[must_use]
pub fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    truncate(line, max)
}

/// Truncate to at most `max` characters, appending `…` when shortened.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_prompt_string() {
        let line = r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-06-27T08:02:55.304Z","message":{"role":"user","content":"do the thing"}}"#;
        let events = parse_events(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].user_text.as_deref(), Some("do the thing"));
        assert!(events[0].timestamp.is_some());
    }

    #[test]
    fn ignores_tool_result_user_event() {
        let line = r#"{"type":"user","uuid":"u2","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#;
        let events = parse_events(line);
        assert_eq!(events[0].user_text, None);
    }

    #[test]
    fn skips_command_injection_prompts() {
        let line = r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#;
        assert_eq!(parse_events(line)[0].user_text, None);
    }

    #[test]
    fn extracts_tool_uses_with_files() {
        let line = r#"{"type":"assistant","uuid":"a1","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/tmp/x.rs"}},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#;
        let uses = &parse_events(line)[0].tool_uses;
        assert_eq!(uses.len(), 2);
        assert_eq!(
            uses[0],
            ToolUse {
                name: "Edit".into(),
                file: Some("/tmp/x.rs".into())
            }
        );
        assert_eq!(
            uses[1],
            ToolUse {
                name: "Bash".into(),
                file: None
            }
        );
    }

    #[test]
    fn skips_unparsable_lines() {
        let text = "not json\n{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}";
        let events = parse_events(text);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn first_line_truncates_with_ellipsis() {
        assert_eq!(first_line("hello\nworld", 100), "hello");
        assert_eq!(first_line("abcdef", 4), "abc…");
        assert_eq!(first_line("  \n  real line", 100), "real line");
    }

    #[test]
    fn skips_pasted_prompt_and_interrupt_marker() {
        let paste = r#"{"type":"user","message":{"role":"user","content":"Administrator in repo ❯ cargo run"}}"#;
        let interrupt = r#"{"type":"user","message":{"role":"user","content":"[Request interrupted by user]"}}"#;
        assert_eq!(parse_events(paste)[0].user_text, None);
        assert_eq!(parse_events(interrupt)[0].user_text, None);
    }

    #[test]
    fn read_events_from_consumes_only_whole_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let full = "{\"type\":\"user\",\"message\":{\"content\":\"one\"}}\n";
        let partial = "{\"type\":\"user\",\"message\":{\"incomplete";
        std::fs::write(&path, format!("{full}{partial}")).unwrap();

        let (events, offset) = read_events_from(&path, 0).unwrap();
        assert_eq!(
            events.len(),
            1,
            "the partial trailing line must be deferred"
        );
        assert_eq!(offset, full.len() as u64);

        // Re-reading from the offset yields nothing until the line completes.
        let (events, offset2) = read_events_from(&path, offset).unwrap();
        assert!(events.is_empty());
        assert_eq!(offset2, offset);
    }

    #[test]
    fn classifies_pr_merge_with_number() {
        let actions = classify_command("gh pr merge 9 --squash --delete-branch");
        assert_eq!(actions, vec![Action::PrMerged(Some(9))]);
    }

    #[test]
    fn classifies_chained_git_commands() {
        let actions = classify_command("git add -A && git commit -m 'x' && git push");
        assert!(actions.contains(&Action::Commit));
        assert!(actions.contains(&Action::Push));
    }

    #[test]
    fn classifies_test_and_build_and_create() {
        assert_eq!(classify_command("cargo nextest run"), vec![Action::Test]);
        assert_eq!(classify_command("just build"), vec![Action::Build]);
        assert_eq!(
            classify_command("gh pr create --base main"),
            vec![Action::PrCreated]
        );
        assert!(classify_command("ls -la").is_empty());
    }

    #[test]
    fn auto_merge_without_number_still_counts() {
        assert_eq!(
            classify_command("gh pr merge --auto --squash"),
            vec![Action::PrMerged(None)]
        );
    }

    #[test]
    fn extracts_commit_subject_and_pr_title() {
        let hl = command_highlights(r#"git add -A && git commit -m "fix: stop the leak""#);
        assert_eq!(hl, vec!["fix: stop the leak".to_owned()]);
        let hl = command_highlights("gh pr create --base main --title 'Add tray mode' --body x");
        assert_eq!(hl, vec!["PR: Add tray mode".to_owned()]);
    }

    #[test]
    fn detects_revert_and_force_push() {
        assert_eq!(classify_command("git revert HEAD"), vec![Action::Revert]);
        // A force push is still a push, additionally flagged.
        let forced = classify_command("git push --force");
        assert!(forced.contains(&Action::Push));
        assert!(forced.contains(&Action::ForcePush));
    }

    #[test]
    fn extracts_actions_from_bash_tool_use() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"git commit -m wip && gh pr merge 12"}}]}}"#;
        let actions = &parse_events(line)[0].actions;
        assert!(actions.contains(&Action::Commit));
        assert!(actions.contains(&Action::PrMerged(Some(12))));
    }

    #[test]
    fn read_events_from_restarts_if_offset_past_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n",
        )
        .unwrap();
        let (events, _) = read_events_from(&path, 9_999).unwrap();
        assert_eq!(events.len(), 1);
    }
}
