//! `hook` — ingest a Claude Code hook event from stdin.
//!
//! Claude Code runs `worklog hook stop` after each assistant turn and
//! `worklog hook session-end` when a session closes, piping the hook's JSON
//! payload (`{ "transcript_path": ..., "session_id": ..., ... }`) on stdin.
//!
//! Whatever happens, the command exits 0 and prints nothing to stdout so it can
//! never block or disrupt the conversation. Problems go to stderr only.

use std::io::Read as _;
use std::path::PathBuf;

use serde_json::Value;
use worklog_core::digest::{EntryKind, TurnEntry, segment};
use worklog_core::store::Store;
use worklog_core::transcript::read_events_from;
use worklog_core::{CoreError, paths::Paths};

use crate::command::Command;

/// Ingest a hook event from stdin.
#[derive(Debug, clap::Args)]
pub(crate) struct HookArgs {
    #[command(subcommand)]
    event: HookEvent,
}

/// Which hook fired.
#[derive(Debug, Clone, Copy, clap::Subcommand)]
enum HookEvent {
    /// A `Stop` hook: one assistant turn just completed.
    Stop,
    /// A `SessionEnd` hook: the session is closing; sweep up any missed turns.
    SessionEnd,
}

impl Command for HookArgs {
    fn run(self) -> miette::Result<()> {
        // Hooks must never fail the conversation: log to stderr, always exit 0.
        if let Err(err) = ingest(self.event) {
            eprintln!("worklog: hook ingest skipped: {err}");
        }
        Ok(())
    }
}

/// The actual work: read stdin, parse the transcript, append new turns.
fn ingest(event: HookEvent) -> Result<(), CoreError> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| CoreError::io("<stdin>", e))?;
    let payload: Value = serde_json::from_str(&input)?;

    let transcript_path = payload
        .get("transcript_path")
        .and_then(Value::as_str)
        .ok_or(CoreError::MissingDir {
            what: "transcript_path in hook payload",
        })?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let paths = Paths::discover()?;
    let store = Store::new(paths.store_dir);

    // Resume from where this session's last hook left off — only the bytes
    // appended since are read, so a long session never re-scans its whole log.
    let mut cursor = store.read_cursor(session_id)?;
    let (events, new_offset) = read_events_from(&PathBuf::from(transcript_path), cursor.offset)?;
    let entries = mark_kind(segment(&mut cursor.context, &events), event);
    store.append_entries(&entries)?;
    cursor.offset = new_offset;
    store.write_cursor(session_id, &cursor)?;
    Ok(())
}

/// Tag entries swept up by a `SessionEnd` hook so the source is traceable.
fn mark_kind(mut entries: Vec<TurnEntry>, event: HookEvent) -> Vec<TurnEntry> {
    if matches!(event, HookEvent::SessionEnd) {
        for entry in &mut entries {
            entry.kind = EntryKind::SessionEnd;
        }
    }
    entries
}
