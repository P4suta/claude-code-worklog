//! Append-only persistence for the continuous (bunpo) stream.
//!
//! The store is a directory tree of newline-delimited JSON, **sharded per
//! session**:
//!
//! ```text
//! <store_dir>/
//!   entries/2026-06-27/<session-id>.ndjson   # one TurnEntry per line
//!   reports/2026-06-27.md                     # rendered daily report (regenerable)
//! ```
//!
//! One file per session so concurrent Claude Code instances never contend on a
//! shared file: a session's hooks only append to its own shard. Reads fan in
//! across a day's shards and skip any unparsable line (e.g. a partial final line
//! caught mid-append). Appends are idempotent by turn `uuid`.

use std::path::{Path, PathBuf};

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::digest::{Context, TurnEntry, local_date};
use crate::error::{CoreError, Result};

/// A per-session ingestion cursor: how far into the transcript we have read, plus
/// the carried segmentation [`Context`] so the next hook resumes incrementally.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SessionCursor {
    /// Byte offset already processed in the session's transcript.
    pub offset: u64,
    /// Carried-forward context (cwd/branch/slug/session).
    #[serde(default)]
    pub context: Context,
}

/// A handle to the on-disk worklog store rooted at a directory.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open (without creating) a store at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The directory holding per-day entry directories.
    #[must_use]
    pub fn entries_dir(&self) -> PathBuf {
        self.root.join("entries")
    }

    /// The directory holding rendered reports.
    #[must_use]
    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("reports")
    }

    /// The directory holding a single day's session shards.
    #[must_use]
    pub fn day_dir(&self, date: Date) -> PathBuf {
        self.entries_dir().join(date.to_string())
    }

    /// The shard file a given session writes for a given day.
    ///
    /// Shards are spread into 256 hash buckets (`<date>/<bb>/<session>.ndjson`) so
    /// that even a day with an enormous number of sessions never piles them all
    /// into a single directory.
    #[must_use]
    pub fn shard_path(&self, date: Date, session_id: &str) -> PathBuf {
        let safe = sanitize(session_id);
        self.day_dir(date)
            .join(bucket(&safe))
            .join(format!("{safe}.ndjson"))
    }

    /// The directory holding per-session ingestion cursors.
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    /// Read a session's ingestion cursor, or the default if none exists yet.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if the cursor file exists but cannot be read, or
    /// [`CoreError::Json`] if it cannot be parsed.
    pub fn read_cursor(&self, session_id: &str) -> Result<SessionCursor> {
        let path = self.cursor_path(session_id);
        match read_optional(&path)? {
            Some(text) => Ok(serde_json::from_str(&text)?),
            None => Ok(SessionCursor::default()),
        }
    }

    /// Persist a session's ingestion cursor.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if the state directory or file cannot be written.
    pub fn write_cursor(&self, session_id: &str, cursor: &SessionCursor) -> Result<()> {
        let dir = self.state_dir();
        std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(&dir, e))?;
        let path = self.cursor_path(session_id);
        let text = serde_json::to_string(cursor)?;
        std::fs::write(&path, text).map_err(|e| CoreError::io(&path, e))
    }

    /// The cursor file for a session.
    fn cursor_path(&self, session_id: &str) -> PathBuf {
        self.state_dir()
            .join(format!("{}.json", sanitize(session_id)))
    }

    /// Append `entries` to their per-session shards, skipping ids already stored.
    ///
    /// Entries are grouped by `(local day, session)`; each group touches exactly
    /// one shard, which only this session ever writes.
    ///
    /// Returns how many entries were newly written.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if a shard cannot be created, read, or written.
    pub fn append_entries(&self, entries: &[TurnEntry]) -> Result<usize> {
        let mut written = 0;
        for (key, group) in group_by_shard(entries) {
            written += self.append_to_shard(key.0, &key.1, &group)?;
        }
        Ok(written)
    }

    /// Append one shard's worth of entries in a single write, deduping by uuid.
    fn append_to_shard(&self, date: Date, session_id: &str, group: &[&TurnEntry]) -> Result<usize> {
        let path = self.shard_path(date, session_id);
        let mut seen = read_shard(&path)?
            .into_iter()
            .map(|e| e.uuid)
            .collect::<HashSet>();

        let mut buf = String::new();
        let mut written = 0;
        for entry in group {
            if !seen.insert(entry.uuid.clone()) {
                continue;
            }
            buf.push_str(&serde_json::to_string(entry)?);
            buf.push('\n');
            written += 1;
        }
        if !buf.is_empty() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| CoreError::io(dir, e))?;
            }
            append_str(&path, &buf)?;
        }
        Ok(written)
    }

    /// Read every entry stored for a local date, merged across all session shards.
    ///
    /// Unparsable lines (e.g. a partial line from a concurrent append) are skipped.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if the day directory cannot be listed or a shard
    /// cannot be read.
    pub fn read_entries(&self, date: Date) -> Result<Vec<TurnEntry>> {
        let dir = self.day_dir(date);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut shards: Vec<PathBuf> = Vec::new();
        collect_shards(&dir, &mut shards)?;
        shards.sort();

        let mut entries = Vec::new();
        for shard in &shards {
            entries.extend(read_shard(shard)?);
        }
        Ok(entries)
    }

    /// Read every entry stored across an inclusive local-date range, merged.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if any day's directory or shard cannot be read.
    pub fn read_range(&self, start: Date, end: Date) -> Result<Vec<TurnEntry>> {
        let mut entries = Vec::new();
        let mut day = start;
        while day <= end {
            entries.extend(self.read_entries(day)?);
            day = day.tomorrow().map_err(|e| CoreError::Parse {
                kind: "date",
                raw: day.to_string(),
                message: e.to_string(),
            })?;
        }
        Ok(entries)
    }

    /// Write a rendered report for `date` and return the file path.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if the directory or file cannot be written.
    pub fn write_report(&self, date: Date, markdown: &str) -> Result<PathBuf> {
        self.write_report_named(&date.to_string(), markdown)
    }

    /// Write a rendered report under an arbitrary `label` (e.g. a period like
    /// `2026-W26`) and return the file path.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if the directory or file cannot be written.
    pub fn write_report_named(&self, label: &str, markdown: &str) -> Result<PathBuf> {
        let dir = self.reports_dir();
        std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(&dir, e))?;
        let path = dir.join(format!("{label}.md"));
        std::fs::write(&path, markdown).map_err(|e| CoreError::io(&path, e))?;
        Ok(path)
    }
}

/// A set of strings (alias to keep the long generic out of the call site).
type HashSet = std::collections::HashSet<String>;

/// Recursively collect every `*.ndjson` shard under `dir` (across hash buckets).
fn collect_shards(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| CoreError::io(dir, e))? {
        let path = entry.map_err(|e| CoreError::io(dir, e))?.path();
        if path.is_dir() {
            collect_shards(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("ndjson") {
            out.push(path);
        }
    }
    Ok(())
}

/// Map a (sanitized) session id to a stable 2-hex-digit bucket via FNV-1a.
///
/// Stable forever (not the std hasher, whose output can change across releases),
/// so a shard written today is still found after a toolchain upgrade.
fn bucket(name: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:02x}", hash & 0xff)
}

/// Read one shard, tolerating (and logging) lines that fail to parse.
fn read_shard(path: &Path) -> Result<Vec<TurnEntry>> {
    let Some(text) = read_optional(path)? else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(entry) => entries.push(entry),
            Err(err) => debug!(?path, %err, "skipping unparsable store line"),
        }
    }
    Ok(entries)
}

/// Group entries by their `(local day, session id)` shard key, preserving order.
fn group_by_shard(entries: &[TurnEntry]) -> Vec<((Date, String), Vec<&TurnEntry>)> {
    let mut order: Vec<(Date, String)> = Vec::new();
    let mut groups: std::collections::HashMap<(Date, String), Vec<&TurnEntry>> =
        std::collections::HashMap::new();
    for entry in entries {
        let key = (local_date(entry.ts), entry.session_id.clone());
        groups
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(entry);
    }
    order
        .into_iter()
        .filter_map(|k| groups.remove(&k).map(|v| (k, v)))
        .collect()
}

/// Make a session id safe to use as a file name.
fn sanitize(session_id: &str) -> String {
    let cleaned: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".to_owned()
    } else {
        cleaned
    }
}

/// Read a file to a string, returning `None` if it does not exist.
fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(path, e)),
    }
}

/// Append a string to a file, creating it if needed.
fn append_str(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| CoreError::io(path, e))?;
    file.write_all(text.as_bytes())
        .map_err(|e| CoreError::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::{Deliverables, EntryKind, TurnEntry};

    fn entry(session: &str, uuid: &str, ts: &str) -> TurnEntry {
        TurnEntry {
            ts: ts.parse().unwrap(),
            session_id: session.into(),
            uuid: uuid.into(),
            cwd: Some("/home/me/proj".into()),
            project: Some("proj".into()),
            git_branch: Some("main".into()),
            slug: Some("topic".into()),
            user_request: Some("do it".into()),
            tools: vec![],
            files_touched: vec![],
            file_churn: vec![],
            interruptions: 0,
            highlights: vec![],
            deliverables: Deliverables::default(),
            kind: EntryKind::Turn,
        }
    }

    #[test]
    fn append_is_idempotent_by_uuid() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        let e = entry("s1", "u1", "2026-06-27T08:00:00Z");

        assert_eq!(store.append_entries(std::slice::from_ref(&e)).unwrap(), 1);
        assert_eq!(store.append_entries(std::slice::from_ref(&e)).unwrap(), 0);

        let read = store.read_entries(local_date(e.ts)).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0], e);
    }

    #[test]
    fn different_sessions_use_separate_shards() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        let a = entry("sessA", "u1", "2026-06-27T08:00:00Z");
        let b = entry("sessB", "u2", "2026-06-27T09:00:00Z");
        let date = local_date(a.ts);

        assert_eq!(store.append_entries(&[a, b]).unwrap(), 2);
        assert!(store.shard_path(date, "sessA").is_file());
        assert!(store.shard_path(date, "sessB").is_file());
        assert_eq!(store.read_entries(date).unwrap().len(), 2);
    }

    #[test]
    fn read_tolerates_a_corrupt_line() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        let e = entry("s1", "u1", "2026-06-27T08:00:00Z");
        let date = local_date(e.ts);
        store.append_entries(std::slice::from_ref(&e)).unwrap();
        // Simulate a partial line landing in the shard during a concurrent append.
        append_str(&store.shard_path(date, "s1"), "{\"ts\":\"2026-").unwrap();

        let read = store.read_entries(date).unwrap();
        assert_eq!(read.len(), 1);
    }

    #[test]
    fn read_missing_date_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        let date: Date = "2020-01-01".parse().unwrap();
        assert!(store.read_entries(date).unwrap().is_empty());
    }

    #[test]
    fn cursor_round_trips_and_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        assert_eq!(store.read_cursor("sess").unwrap().offset, 0);

        let cursor = SessionCursor {
            offset: 4096,
            context: Context::default(),
        };
        store.write_cursor("sess", &cursor).unwrap();
        assert_eq!(store.read_cursor("sess").unwrap().offset, 4096);
    }
}
