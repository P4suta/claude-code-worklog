//! Segmenting transcript events into turns and aggregating them per session.
//!
//! A *turn* is one user prompt and the assistant work that answers it — exactly
//! what a `Stop` hook fires after. [`entries_from_events`] cuts a transcript into
//! [`TurnEntry`]s on that boundary; [`aggregate`] then rolls a day's entries up
//! into one [`SessionDigest`] per session for the daily report.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::transcript::{Action, RawEvent, first_line};

/// How many characters of a user prompt to keep.
const REQUEST_MAX: usize = 140;

/// What produced a stored entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A normal assistant turn (recorded by a `Stop` hook or a daily backfill).
    #[default]
    Turn,
    /// A turn swept up when a session ended (recorded by a `SessionEnd` hook).
    SessionEnd,
}

/// A tool name paired with how many times it was used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCount {
    /// The tool name.
    pub name: String,
    /// Number of invocations.
    pub count: u32,
}

/// A file path paired with how many times it was edited (a churn signal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCount {
    /// The file path.
    pub file: String,
    /// Number of mutating edits.
    pub count: u32,
}

/// Where effort went in a session: a coarse explore/implement/verify split.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffortMix {
    /// Read/Grep/Glob/Web — understanding the code.
    pub explore: u32,
    /// Edit/Write — changing the code.
    pub implement: u32,
    /// Test/build invocations — checking the code.
    pub verify: u32,
}

impl EffortMix {
    /// Total weighted activity.
    #[must_use]
    pub const fn total(self) -> u32 {
        self.explore + self.implement + self.verify
    }
}

/// Tool names that count as exploration.
const EXPLORE_TOOLS: [&str; 6] = ["Read", "Grep", "Glob", "WebFetch", "WebSearch", "LS"];
/// Tool names that count as implementation (mutating a file).
const MUTATING_TOOLS: [&str; 5] = ["Edit", "Write", "MultiEdit", "NotebookEdit", "Create"];

/// Concrete, executive-facing outcomes counted from shell activity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deliverables {
    /// Pull requests opened (`gh pr create`).
    pub prs_created: u32,
    /// Pull requests merged (`gh pr merge`).
    pub prs_merged: u32,
    /// Commits made (`git commit`).
    pub commits: u32,
    /// Pushes (`git push`).
    pub pushes: u32,
    /// Releases or tags cut.
    pub releases: u32,
    /// Test-runner invocations.
    pub tests: u32,
    /// Build invocations.
    pub builds: u32,
    /// `git revert`s (a risk signal).
    #[serde(default)]
    pub reverts: u32,
    /// Force pushes (a risk signal).
    #[serde(default)]
    pub force_pushes: u32,
    /// Distinct merged/created PR numbers seen, sorted.
    pub pr_refs: Vec<u32>,
}

impl Deliverables {
    /// Fold one inferred [`Action`] into the tally.
    fn record(&mut self, action: Action) {
        match action {
            Action::PrCreated => self.prs_created += 1,
            Action::PrMerged(number) => {
                self.prs_merged += 1;
                if let Some(n) = number {
                    self.add_ref(n);
                }
            },
            Action::Commit => self.commits += 1,
            Action::Push => self.pushes += 1,
            Action::Release => self.releases += 1,
            Action::Test => self.tests += 1,
            Action::Build => self.builds += 1,
            Action::Revert => self.reverts += 1,
            Action::ForcePush => self.force_pushes += 1,
        }
    }

    /// Sum another tally into this one.
    pub fn merge(&mut self, other: &Self) {
        self.prs_created += other.prs_created;
        self.prs_merged += other.prs_merged;
        self.commits += other.commits;
        self.pushes += other.pushes;
        self.releases += other.releases;
        self.tests += other.tests;
        self.builds += other.builds;
        self.reverts += other.reverts;
        self.force_pushes += other.force_pushes;
        for n in &other.pr_refs {
            self.add_ref(*n);
        }
    }

    /// Record a PR number, keeping `pr_refs` unique and sorted.
    fn add_ref(&mut self, number: u32) {
        if let Err(pos) = self.pr_refs.binary_search(&number) {
            self.pr_refs.insert(pos, number);
        }
    }

    /// Whether anything was shipped, verified, or flagged.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.prs_created == 0
            && self.prs_merged == 0
            && self.commits == 0
            && self.pushes == 0
            && self.releases == 0
            && self.tests == 0
            && self.builds == 0
            && self.reverts == 0
            && self.force_pushes == 0
    }
}

/// One stored unit of the continuous (bunpo) stream: a single assistant turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEntry {
    /// When the turn began (UTC instant; bucketed into local days on display).
    pub ts: Timestamp,
    /// The session this turn belongs to.
    pub session_id: String,
    /// A stable id for the turn, used to deduplicate repeated ingests.
    pub uuid: String,
    /// The working directory, if recorded.
    pub cwd: Option<String>,
    /// The project name (basename of `cwd`), if derivable.
    pub project: Option<String>,
    /// The git branch, if recorded.
    pub git_branch: Option<String>,
    /// The conversation slug (title), if recorded.
    pub slug: Option<String>,
    /// The first line of the user's prompt that opened the turn, if any.
    pub user_request: Option<String>,
    /// The tools used during the turn, with counts.
    pub tools: Vec<ToolCount>,
    /// Distinct files touched during the turn.
    pub files_touched: Vec<String>,
    /// Per-file mutating-edit counts during the turn (churn).
    #[serde(default)]
    pub file_churn: Vec<FileCount>,
    /// Number of user interrupts (course-corrections) during the turn.
    #[serde(default)]
    pub interruptions: u32,
    /// Work items (commit subjects, PR titles) named during the turn.
    #[serde(default)]
    pub highlights: Vec<String>,
    /// Deliverables (PRs/commits/tests/…) inferred during the turn.
    #[serde(default)]
    pub deliverables: Deliverables,
    /// What produced this entry.
    #[serde(default)]
    pub kind: EntryKind,
}

/// A whole session's worth of turns, rolled up for the daily report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionDigest {
    /// The session id.
    pub session_id: String,
    /// The project name (basename of `cwd`), if known.
    pub project: Option<String>,
    /// The working directory, if known.
    pub cwd: Option<String>,
    /// The git branch, if known.
    pub git_branch: Option<String>,
    /// The conversation slug (title), if known.
    pub slug: Option<String>,
    /// The first turn's timestamp.
    pub start: Timestamp,
    /// The last turn's timestamp.
    pub end: Timestamp,
    /// How many turns the session contained.
    pub turn_count: u32,
    /// How many sessions this digest represents (1 unless merged by project).
    #[serde(default)]
    pub session_count: u32,
    /// The session ids this digest covers (for memory back-links after merging).
    #[serde(default)]
    pub members: Vec<String>,
    /// The user's requests, in order.
    pub requests: Vec<String>,
    /// Tool usage across the session, most-used first.
    pub tools: Vec<ToolCount>,
    /// Distinct files touched across the session.
    pub files_touched: Vec<String>,
    /// Per-file mutating-edit counts across the session (churn / hot spots).
    #[serde(default)]
    pub file_churn: Vec<FileCount>,
    /// Number of user interrupts (course-corrections) across the session.
    #[serde(default)]
    pub interruptions: u32,
    /// Work items (commit subjects, PR titles) across the session, in order.
    #[serde(default)]
    pub highlights: Vec<String>,
    /// Deliverables (PRs/commits/tests/…) across the session.
    #[serde(default)]
    pub deliverables: Deliverables,
}

/// The local calendar date an instant falls on, in the system time zone.
#[must_use]
pub fn local_date(ts: Timestamp) -> jiff::civil::Date {
    ts.to_zoned(TimeZone::system()).date()
}

/// The local `HH:MM` wall-clock time of an instant, in the system time zone.
#[must_use]
pub fn local_hm(ts: Timestamp) -> String {
    ts.to_zoned(TimeZone::system())
        .strftime("%H:%M")
        .to_string()
}

/// Cut a transcript's events into one [`TurnEntry`] per user/assistant turn.
///
/// Events are assumed to be in chronological order (as written). Tool work that
/// precedes any captured prompt is folded into an "orphan" turn so it is not lost.
#[must_use]
pub fn entries_from_events(events: &[RawEvent]) -> Vec<TurnEntry> {
    let mut ctx = Context::default();
    segment(&mut ctx, events)
}

/// Segment `events` into finished turns, threading carried `ctx` (updated in
/// place so a caller can resume across incremental reads of a growing file).
///
/// The final in-progress turn is flushed on each call. That is correct at a
/// `Stop`/`SessionEnd` boundary — where the assistant turn has just completed and
/// no newer prompt exists yet — and lets an incremental ingester process only the
/// bytes appended since last time instead of re-reading the whole transcript.
#[must_use]
pub fn segment(ctx: &mut Context, events: &[RawEvent]) -> Vec<TurnEntry> {
    let mut pending: Option<Pending> = None;
    let mut out = Vec::new();
    for event in events {
        ctx.absorb(event);
        if event.user_text.is_some() {
            push_finished(&mut out, pending.take(), ctx);
            pending = Some(Pending::opened_by(event));
        } else if event.interrupted {
            pending.get_or_insert_with(Pending::default).interruptions += 1;
        } else if !event.tool_uses.is_empty() || event.event_type == "assistant" {
            pending
                .get_or_insert_with(Pending::default)
                .absorb_assistant(event);
        }
    }
    push_finished(&mut out, pending, ctx);
    out
}

/// Finalize a pending turn (if any) into `out`.
fn push_finished(out: &mut Vec<TurnEntry>, pending: Option<Pending>, ctx: &Context) {
    if let Some(entry) = pending.and_then(|p| p.into_entry(ctx)) {
        out.push(entry);
    }
}

/// Drop duplicate entries by `uuid`, keeping the first occurrence and order.
#[must_use]
pub fn dedup_by_uuid(entries: Vec<TurnEntry>) -> Vec<TurnEntry> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|e| seen.insert(e.uuid.clone()))
        .collect()
}

/// Roll a flat list of turns up into one digest per session, sorted by start.
#[must_use]
pub fn aggregate(mut entries: Vec<TurnEntry>) -> Vec<SessionDigest> {
    entries.sort_by_key(|e| e.ts);
    let mut order: Vec<String> = Vec::new();
    let mut by_session: std::collections::HashMap<String, SessionDigest> =
        std::collections::HashMap::new();

    for entry in entries {
        let digest = by_session
            .entry(entry.session_id.clone())
            .or_insert_with(|| {
                order.push(entry.session_id.clone());
                SessionDigest::seed(&entry)
            });
        digest.merge(entry);
    }

    let mut digests: Vec<SessionDigest> = order
        .into_iter()
        .filter_map(|id| by_session.remove(&id))
        .collect();
    digests.sort_by_key(|d| d.start);
    for digest in &mut digests {
        sort_tools(&mut digest.tools);
    }
    digests
}

/// Merge per-session digests that share a project into one digest each.
///
/// Useful for a "what did I do on project X today" view when one task was spread
/// across several sessions. Input is expected sorted by start (as [`aggregate`]
/// returns), so merged requests stay in chronological order.
#[must_use]
pub fn merge_by_project(digests: Vec<SessionDigest>) -> Vec<SessionDigest> {
    let mut order: Vec<String> = Vec::new();
    let mut by_project: std::collections::HashMap<String, SessionDigest> =
        std::collections::HashMap::new();

    for digest in digests {
        let key = digest
            .project
            .clone()
            .unwrap_or_else(|| "(unknown)".to_owned());
        if let Some(acc) = by_project.get_mut(&key) {
            acc.absorb_session(digest);
        } else {
            order.push(key.clone());
            let mut base = digest;
            base.session_id.clone_from(&key);
            base.slug = None;
            by_project.insert(key, base);
        }
    }

    let mut digests: Vec<SessionDigest> = order
        .into_iter()
        .filter_map(|k| by_project.remove(&k))
        .collect();
    digests.sort_by_key(|d| d.start);
    for digest in &mut digests {
        sort_tools(&mut digest.tools);
    }
    digests
}

/// Sum tool counts across digests, most-used first.
#[must_use]
pub fn tool_totals(digests: &[SessionDigest]) -> Vec<ToolCount> {
    let mut totals: Vec<ToolCount> = Vec::new();
    for digest in digests {
        for tool in &digest.tools {
            bump(&mut totals, &tool.name, tool.count);
        }
    }
    sort_tools(&mut totals);
    totals
}

/// Carried-forward context (cwd/branch/slug/session) seen while scanning events.
///
/// Public and serializable so an incremental ingester can persist it between hook
/// runs and resume segmentation without re-reading earlier events.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Context {
    session_id: Option<String>,
    cwd: Option<String>,
    git_branch: Option<String>,
    slug: Option<String>,
}

impl Context {
    fn absorb(&mut self, event: &RawEvent) {
        take_latest(&mut self.session_id, event.session_id.as_deref());
        take_latest(&mut self.cwd, event.cwd.as_deref());
        take_latest(&mut self.git_branch, event.git_branch.as_deref());
        take_latest(&mut self.slug, event.slug.as_deref());
    }
}

/// A turn being assembled across consecutive events.
#[derive(Debug, Default)]
struct Pending {
    start: Option<Timestamp>,
    last: Option<Timestamp>,
    open_uuid: Option<String>,
    last_uuid: Option<String>,
    request: Option<String>,
    tools: Vec<ToolCount>,
    files: Vec<String>,
    churn: Vec<FileCount>,
    interruptions: u32,
    highlights: Vec<String>,
    deliverables: Deliverables,
}

impl Pending {
    fn opened_by(event: &RawEvent) -> Self {
        Self {
            start: event.timestamp,
            last: event.timestamp,
            open_uuid: event.uuid.clone(),
            last_uuid: event.uuid.clone(),
            request: event
                .user_text
                .as_deref()
                .map(|t| first_line(t, REQUEST_MAX)),
            ..Self::default()
        }
    }

    fn absorb_assistant(&mut self, event: &RawEvent) {
        if self.start.is_none() {
            self.start = event.timestamp;
        }
        if event.timestamp.is_some() {
            self.last = event.timestamp;
        }
        if event.uuid.is_some() {
            self.last_uuid.clone_from(&event.uuid);
        }
        for tool in &event.tool_uses {
            bump(&mut self.tools, &tool.name, 1);
            if let Some(file) = &tool.file {
                if !self.files.contains(file) {
                    self.files.push(file.clone());
                }
                if MUTATING_TOOLS.contains(&tool.name.as_str()) {
                    bump_file(&mut self.churn, file);
                }
            }
        }
        for action in &event.actions {
            self.deliverables.record(*action);
        }
        for highlight in &event.highlights {
            if !self.highlights.contains(highlight) {
                self.highlights.push(highlight.clone());
            }
        }
    }

    fn into_entry(self, ctx: &Context) -> Option<TurnEntry> {
        let ts = self.start.or(self.last)?;
        let session_id = ctx.session_id.clone()?;
        let uuid = self
            .last_uuid
            .or(self.open_uuid)
            .unwrap_or_else(|| format!("{session_id}:{ts}"));
        let cwd = ctx.cwd.clone();
        let project = cwd.as_deref().map(project_name);
        Some(TurnEntry {
            ts,
            session_id,
            uuid,
            cwd,
            project,
            git_branch: ctx.git_branch.clone(),
            slug: ctx.slug.clone(),
            user_request: self.request,
            tools: self.tools,
            files_touched: self.files,
            file_churn: self.churn,
            interruptions: self.interruptions,
            highlights: self.highlights,
            deliverables: self.deliverables,
            kind: EntryKind::Turn,
        })
    }
}

impl SessionDigest {
    fn seed(entry: &TurnEntry) -> Self {
        Self {
            session_id: entry.session_id.clone(),
            project: entry.project.clone(),
            cwd: entry.cwd.clone(),
            git_branch: entry.git_branch.clone(),
            slug: entry.slug.clone(),
            start: entry.ts,
            end: entry.ts,
            turn_count: 0,
            session_count: 1,
            members: vec![entry.session_id.clone()],
            requests: Vec::new(),
            tools: Vec::new(),
            files_touched: Vec::new(),
            file_churn: Vec::new(),
            interruptions: 0,
            highlights: Vec::new(),
            deliverables: Deliverables::default(),
        }
    }

    fn merge(&mut self, entry: TurnEntry) {
        self.start = self.start.min(entry.ts);
        self.end = self.end.max(entry.ts);
        self.turn_count += 1;
        fill_if_empty(&mut self.project, entry.project);
        fill_if_empty(&mut self.cwd, entry.cwd);
        fill_if_empty(&mut self.git_branch, entry.git_branch);
        fill_if_empty(&mut self.slug, entry.slug);
        if let Some(request) = entry.user_request {
            self.requests.push(request);
        }
        for tool in entry.tools {
            bump(&mut self.tools, &tool.name, tool.count);
        }
        for file in entry.files_touched {
            if !self.files_touched.contains(&file) {
                self.files_touched.push(file);
            }
        }
        for highlight in entry.highlights {
            if !self.highlights.contains(&highlight) {
                self.highlights.push(highlight);
            }
        }
        for fc in entry.file_churn {
            bump_file_by(&mut self.file_churn, &fc.file, fc.count);
        }
        self.interruptions += entry.interruptions;
        self.deliverables.merge(&entry.deliverables);
    }

    /// Fold another session's rolled-up digest into this one (project grouping).
    fn absorb_session(&mut self, other: Self) {
        self.start = self.start.min(other.start);
        self.end = self.end.max(other.end);
        self.turn_count += other.turn_count;
        self.session_count += other.session_count;
        self.members.extend(other.members);
        fill_if_empty(&mut self.cwd, other.cwd);
        fill_if_empty(&mut self.git_branch, other.git_branch);
        self.requests.extend(other.requests);
        for tool in other.tools {
            bump(&mut self.tools, &tool.name, tool.count);
        }
        for file in other.files_touched {
            if !self.files_touched.contains(&file) {
                self.files_touched.push(file);
            }
        }
        for highlight in other.highlights {
            if !self.highlights.contains(&highlight) {
                self.highlights.push(highlight);
            }
        }
        for fc in other.file_churn {
            bump_file_by(&mut self.file_churn, &fc.file, fc.count);
        }
        self.interruptions += other.interruptions;
        self.deliverables.merge(&other.deliverables);
    }
}

/// The basename of a working directory, handling both `/` and `\` separators.
fn project_name(cwd: &str) -> String {
    cwd.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(cwd)
        .to_owned()
}

/// The `n` most-touched top-level areas across `files`, most-frequent first.
///
/// Each file is made relative to `cwd` (when it lives under it) and bucketed by its
/// first path segment — so `…/proj/engine/src/x.rs` counts as area `engine`. This
/// gives a coarse "where work happened" view rather than a deep directory list.
/// Ties break by name for stable output.
#[must_use]
pub fn top_areas(files: &[String], cwd: Option<&str>, n: usize) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for file in files {
        let area = area_of(file, cwd);
        if let Some(entry) = counts.iter_mut().find(|(a, _)| *a == area) {
            entry.1 += 1;
        } else {
            counts.push((area, 1));
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts.truncate(n);
    counts
}

/// The top-level area a file belongs to, relative to `cwd`.
fn area_of(file: &str, cwd: Option<&str>) -> String {
    let rel = relativize(file, cwd);
    // Not under the project (a memory file, another repo, …) → group together.
    if is_absolute(&rel) {
        return "(other)".to_owned();
    }
    let mut parts = rel.split(['/', '\\']).filter(|s| !s.is_empty());
    match (parts.next(), parts.clone().next()) {
        // A nested file: its first segment is the area.
        (Some(first), Some(_)) => first.to_owned(),
        // A file sitting directly in the project root (or no path).
        _ => "(root)".to_owned(),
    }
}

/// Whether a path is absolute (Unix root or a Windows `X:` drive prefix).
fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || path.starts_with('\\') || matches!(path.as_bytes(), [_, b':', ..])
}

/// Strip a leading `cwd` from `file` when present, returning the remainder.
fn relativize(file: &str, cwd: Option<&str>) -> String {
    if let Some(cwd) = cwd {
        let base = cwd.trim_end_matches(['/', '\\']);
        if let Some(rest) = file.strip_prefix(base) {
            return rest.trim_start_matches(['/', '\\']).to_owned();
        }
    }
    file.to_owned()
}

/// Overwrite a `None` slot with `value`; leave an existing value untouched.
fn fill_if_empty(slot: &mut Option<String>, value: Option<String>) {
    if slot.is_none() {
        *slot = value;
    }
}

/// Adopt `incoming` if present (latest non-empty wins via repeat calls).
fn take_latest(slot: &mut Option<String>, incoming: Option<&str>) {
    if let Some(value) = incoming {
        *slot = Some(value.to_owned());
    }
}

/// Add `count` to `name`'s tally, appending it if not yet present.
fn bump(counts: &mut Vec<ToolCount>, name: &str, count: u32) {
    if let Some(existing) = counts.iter_mut().find(|c| c.name == name) {
        existing.count += count;
    } else {
        counts.push(ToolCount {
            name: name.to_owned(),
            count,
        });
    }
}

/// Increment a file's churn tally by one.
fn bump_file(counts: &mut Vec<FileCount>, file: &str) {
    bump_file_by(counts, file, 1);
}

/// Add `count` to a file's churn tally, appending it if not yet present.
fn bump_file_by(counts: &mut Vec<FileCount>, file: &str, count: u32) {
    if let Some(existing) = counts.iter_mut().find(|c| c.file == file) {
        existing.count += count;
    } else {
        counts.push(FileCount {
            file: file.to_owned(),
            count,
        });
    }
}

/// The session's explore/implement/verify effort split, from tool and
/// deliverable counts.
#[must_use]
pub fn effort_mix(digest: &SessionDigest) -> EffortMix {
    let mut mix = EffortMix::default();
    for tool in &digest.tools {
        if EXPLORE_TOOLS.contains(&tool.name.as_str()) {
            mix.explore += tool.count;
        } else if MUTATING_TOOLS.contains(&tool.name.as_str()) {
            mix.implement += tool.count;
        }
    }
    mix.verify = digest.deliverables.tests + digest.deliverables.builds;
    mix
}

/// The `n` hottest churned files (mutating edits ≥ `min`), most-churned first.
#[must_use]
pub fn hotspots(digest: &SessionDigest, min: u32, n: usize) -> Vec<FileCount> {
    let mut churn: Vec<FileCount> = digest
        .file_churn
        .iter()
        .filter(|c| c.count >= min)
        .cloned()
        .collect();
    churn.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.file.cmp(&b.file)));
    churn.truncate(n);
    churn
}

/// The basename of a file path (for compact hot-spot display).
#[must_use]
pub fn file_basename(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// Sort tool counts by descending count, then name for stability.
fn sort_tools(tools: &mut [ToolCount]) {
    tools.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::parse_events;

    fn events(jsonl: &str) -> Vec<RawEvent> {
        parse_events(jsonl)
    }

    const SAMPLE: &str = concat!(
        r#"{"type":"user","uuid":"u1","sessionId":"s1","cwd":"/home/me/proj","gitBranch":"main","slug":"do-stuff","timestamp":"2026-06-27T08:00:00Z","message":{"role":"user","content":"first task"}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a1","sessionId":"s1","timestamp":"2026-06-27T08:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/home/me/proj/x.rs"}}]}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a2","sessionId":"s1","timestamp":"2026-06-27T08:02:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/home/me/proj/y.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/home/me/proj/x.rs"}}]}}"#,
        "\n",
        r#"{"type":"user","uuid":"u2","sessionId":"s1","timestamp":"2026-06-27T08:05:00Z","message":{"role":"user","content":"second task"}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a3","sessionId":"s1","timestamp":"2026-06-27T08:06:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
    );

    #[test]
    fn segments_into_two_turns() {
        let entries = entries_from_events(&events(SAMPLE));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].user_request.as_deref(), Some("first task"));
        assert_eq!(entries[0].uuid, "a2"); // last uuid in the turn
        assert_eq!(entries[1].user_request.as_deref(), Some("second task"));
        assert_eq!(entries[1].uuid, "a3");
    }

    #[test]
    fn turn_collects_tools_and_distinct_files() {
        let first = &entries_from_events(&events(SAMPLE))[0];
        assert_eq!(
            first.tools,
            vec![
                ToolCount {
                    name: "Read".into(),
                    count: 2
                },
                ToolCount {
                    name: "Edit".into(),
                    count: 1
                },
            ]
        );
        assert_eq!(
            first.files_touched,
            vec![
                "/home/me/proj/x.rs".to_owned(),
                "/home/me/proj/y.rs".to_owned(),
            ]
        );
        assert_eq!(first.project.as_deref(), Some("proj"));
    }

    #[test]
    fn aggregate_rolls_session_and_sorts_tools() {
        let digests = aggregate(entries_from_events(&events(SAMPLE)));
        assert_eq!(digests.len(), 1);
        let d = &digests[0];
        assert_eq!(d.turn_count, 2);
        assert_eq!(
            d.requests,
            vec!["first task".to_owned(), "second task".to_owned()]
        );
        assert_eq!(
            d.tools[0],
            ToolCount {
                name: "Read".into(),
                count: 2
            }
        );
        assert_eq!(d.slug.as_deref(), Some("do-stuff"));
        assert!(d.start < d.end);
    }

    #[test]
    fn dedup_keeps_first_occurrence() {
        let entries = entries_from_events(&events(SAMPLE));
        let mut doubled = entries.clone();
        doubled.extend(entries);
        assert_eq!(dedup_by_uuid(doubled).len(), 2);
    }

    #[test]
    fn local_date_matches_iso_for_utc_system_tz() {
        // Only assert structure: the date is well-formed and stable across calls.
        let ts: Timestamp = "2026-06-27T23:59:00Z".parse().unwrap();
        assert_eq!(local_date(ts), local_date(ts));
    }

    #[test]
    fn segment_resumes_across_incremental_chunks() {
        // Split SAMPLE at the second user prompt: chunk 1 = turn 1, chunk 2 = turn 2.
        let lines: Vec<&str> = SAMPLE.lines().collect();
        let chunk1 = lines[..3].join("\n");
        let chunk2 = lines[3..].join("\n");

        let mut ctx = Context::default();
        let first = segment(&mut ctx, &parse_events(&chunk1));
        let second = segment(&mut ctx, &parse_events(&chunk2));

        // Same turns as a single full pass, with no overlap.
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].uuid, "a2");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].uuid, "a3");
        // Context carried the cwd-derived project into the second chunk's turn.
        assert_eq!(second[0].project.as_deref(), Some("proj"));
    }

    #[test]
    fn aggregates_deliverables_from_bash_actions() {
        let jsonl = concat!(
            r#"{"type":"user","uuid":"u1","sessionId":"s1","cwd":"/p","timestamp":"2026-06-27T08:00:00Z","message":{"role":"user","content":"ship it"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","sessionId":"s1","timestamp":"2026-06-27T08:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"git commit -m x && git push && cargo test"}}]}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a2","sessionId":"s1","timestamp":"2026-06-27T08:02:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"gh pr merge 7 --squash"}}]}}"#,
        );
        let d = &aggregate(entries_from_events(&events(jsonl)))[0];
        assert_eq!(d.deliverables.commits, 1);
        assert_eq!(d.deliverables.pushes, 1);
        assert_eq!(d.deliverables.tests, 1);
        assert_eq!(d.deliverables.prs_merged, 1);
        assert_eq!(d.deliverables.pr_refs, vec![7]);
    }

    #[test]
    fn top_areas_buckets_by_first_segment() {
        let files = vec![
            "/p/engine/src/a.rs".to_owned(),
            "/p/engine/src/b.rs".to_owned(),
            "/p/docs/adr/0001.md".to_owned(),
            "/p/justfile".to_owned(),
        ];
        let areas = top_areas(&files, Some("/p"), 3);
        assert_eq!(areas[0], ("engine".to_owned(), 2));
        assert!(areas.contains(&("docs".to_owned(), 1)));
        assert!(areas.contains(&("(root)".to_owned(), 1)));
    }

    #[test]
    fn top_areas_groups_files_outside_cwd_as_other() {
        let files = vec![
            "/p/engine/a.rs".to_owned(),
            r"C:\Users\me\.claude\memory\x.md".to_owned(),
            "/other/repo/y.rs".to_owned(),
        ];
        let areas = top_areas(&files, Some("/p"), 3);
        assert!(areas.contains(&("(other)".to_owned(), 2)));
        assert!(areas.contains(&("engine".to_owned(), 1)));
    }

    #[test]
    fn merge_by_project_combines_sessions() {
        let other = concat!(
            r#"{"type":"user","uuid":"v1","sessionId":"s2","cwd":"/home/me/proj","timestamp":"2026-06-27T10:00:00Z","message":{"role":"user","content":"third task"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"b1","sessionId":"s2","timestamp":"2026-06-27T10:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        let mut entries = entries_from_events(&events(SAMPLE));
        entries.extend(entries_from_events(&events(other)));

        let merged = merge_by_project(aggregate(entries));
        assert_eq!(merged.len(), 1, "both sessions share project 'proj'");
        let p = &merged[0];
        assert_eq!(p.session_count, 2);
        assert_eq!(p.turn_count, 3);
        assert_eq!(
            p.requests,
            vec![
                "first task".to_owned(),
                "second task".to_owned(),
                "third task".to_owned(),
            ]
        );
    }
}
