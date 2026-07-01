//! `worklog-core` — turn Claude Code session logs into work reports.
//!
//! Claude Code records every session as a transcript: a newline-delimited JSON
//! file under `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`. Each line is
//! one event (a user prompt, an assistant turn with tool calls, a system note, …)
//! stamped with a UTC timestamp. This crate reads those transcripts and distills
//! them into:
//!
//! - a **continuous stream** (*bunpo*) of one [`TurnEntry`] per assistant turn,
//!   persisted append-only in a [`store::Store`], and
//! - a **daily report** (*nippo*) aggregated into [`SessionDigest`]s and rendered
//!   to Markdown by [`report`].
//!
//! No network and no LLM are involved: everything is structured extraction. An
//! optional [`summarize::Summarizer`] seam exists for a future LLM pass, but the
//! default [`summarize::NullSummarizer`] does nothing.
//!
//! ## Privacy
//!
//! Extraction is deliberately shallow. Only user prompt text (first line,
//! truncated), tool *names*, and touched file paths are kept. Assistant reasoning
//! and raw tool output — the usual homes for secrets — are never copied into the
//! store or a report.
//!
//! ## Modules
//!
//! - [`paths`] — locate `~/.claude`, the transcripts, and the store.
//! - [`transcript`] — parse a `.jsonl` file into [`transcript::RawEvent`]s.
//! - [`digest`] — segment events into [`TurnEntry`]s and aggregate [`SessionDigest`]s.
//! - [`store`] — append-only persistence of the bunpo stream.
//! - [`report`] — render a daily report to Markdown.
//! - [`memory`] — optional back-links from Claude Code memory files.
//! - [`summarize`] — the (default no-op) summarizer seam.

pub mod baseline;
pub mod digest;
pub mod error;
pub mod memory;
pub mod paths;
pub mod report;
pub mod store;
pub mod summarize;
pub mod transcript;

pub use baseline::{Baseline, baseline_from_entries};
pub use digest::{Context, Deliverables, EntryKind, SessionDigest, ToolCount, TurnEntry};
pub use error::{CoreError, Result};
pub use report::{RenderOptions, Style, TrendData};
pub use transcript::Action;
