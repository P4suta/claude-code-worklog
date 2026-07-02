//! `period` — weekly / monthly / range roll-up with a prior-period trend.
//!
//! Aggregates every turn in a date range through the same machinery as `daily`
//! (so the outcome metrics and process spice roll up for free), and — unless
//! `--no-trend` — compares against the immediately preceding period of equal
//! length to show prior-period deltas.

use std::collections::HashMap;
use std::path::PathBuf;

use jiff::ToSpan as _;
use jiff::civil::Date;
use miette::{IntoDiagnostic as _, miette};
use worklog_core::digest::{
    aggregate, dedup_by_uuid, entries_from_events, local_date, merge_by_project,
};
use worklog_core::report::{RenderOptions, Style, TrendData, render};
use worklog_core::store::Store;
use worklog_core::summarize::NullSummarizer;
use worklog_core::transcript::read_events;
use worklog_core::{Deliverables, SessionDigest, TurnEntry, memory, paths::Paths};

use crate::command::Command;
use crate::commands::{StyleArg, parse_date};

/// Weekly / monthly / range report with a prior-period trend.
#[derive(Debug, clap::Args)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these are independent CLI flags, not a state machine"
)]
pub(crate) struct PeriodArgs {
    /// The ISO week (Mon–Sun) containing `--date` (the default span).
    #[arg(long, conflicts_with_all = ["month", "since", "until"])]
    week: bool,

    /// The calendar month containing `--date`.
    #[arg(long, conflicts_with_all = ["week", "since", "until"])]
    month: bool,

    /// Range start (inclusive), `YYYY-MM-DD`; use with `--until`.
    #[arg(long, requires = "until")]
    since: Option<String>,

    /// Range end (inclusive), `YYYY-MM-DD`; use with `--since`.
    #[arg(long, requires = "since")]
    until: Option<String>,

    /// Anchor date for `--week` / `--month` (`today`, `yesterday`, `YYYY-MM-DD`).
    #[arg(long, default_value = "today")]
    date: String,

    /// Limit to a single project's working directory (default: all projects).
    #[arg(long, conflicts_with = "all", value_hint = clap::ValueHint::DirPath)]
    project: Option<PathBuf>,

    /// Scan all projects (the default; accepted for explicitness).
    #[arg(long)]
    all: bool,

    /// Also write the rendered report to this path.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    out: Option<PathBuf>,

    /// Use only the stored stream; do not re-scan transcripts.
    #[arg(long)]
    no_backfill: bool,

    /// Print only; do not save the report into the store.
    #[arg(long)]
    no_store: bool,

    /// Do not compute the prior-period trend.
    #[arg(long)]
    no_trend: bool,

    /// Report style: `exec` (default) or `detail`.
    #[arg(long, value_enum, default_value_t = StyleArg::Exec)]
    style: StyleArg,
}

/// A resolved reporting span and how to talk about it.
struct Span {
    start: Date,
    end: Date,
    /// Filename label, e.g. `2026-W26` / `2026-06` / `2026-06-21_2026-06-27`.
    label: String,
    /// Human-readable report title.
    title: String,
    /// Trend comparison label (week / month / range).
    trend_label: String,
    /// Whether the prior period is the previous calendar month.
    monthly: bool,
}

impl Command for PeriodArgs {
    fn run(self) -> miette::Result<()> {
        let span = self.resolve_span()?;
        let paths = Paths::discover().into_diagnostic()?;
        let store = Store::new(paths.store_dir.clone());
        let memory = memory::descriptions_by_session(&paths.projects_dir);
        let style: Style = self.style.into();

        let digests = self.digests(&paths, &store, span.start, span.end, style)?;
        let trend = self.trend(&paths, &store, &span)?;

        let opts = RenderOptions {
            memory: &memory,
            summarizer: &NullSummarizer,
            style,
            trend: trend.as_ref(),
            baseline: None,
        };
        let markdown = render(&span.title, &digests, &opts);

        if !self.no_store {
            let path = store
                .write_report_named(&span.label, &markdown)
                .into_diagnostic()?;
            if self.out.is_none() {
                eprintln!("saved report to {}", path.display());
            }
        }
        if let Some(out) = &self.out {
            std::fs::write(out, &markdown).into_diagnostic()?;
        }
        print!("{markdown}");
        Ok(())
    }
}

impl PeriodArgs {
    /// Aggregate the turns in `[start, end]` into render-ready digests.
    fn digests(
        &self,
        paths: &Paths,
        store: &Store,
        start: Date,
        end: Date,
        style: Style,
    ) -> miette::Result<Vec<SessionDigest>> {
        let entries = self.gather(paths, store, start, end)?;
        let digests = aggregate(entries);
        Ok(match style {
            Style::Exec => merge_by_project(digests),
            Style::Detail => digests,
        })
    }

    /// Build the prior-period trend, unless disabled or in detail view.
    fn trend(
        &self,
        paths: &Paths,
        store: &Store,
        span: &Span,
    ) -> miette::Result<Option<TrendData>> {
        if self.no_trend || matches!(self.style, StyleArg::Detail) {
            return Ok(None);
        }
        let (prev_start, prev_end) = previous_span(span)?;
        let prev = merge_by_project(aggregate(self.gather(paths, store, prev_start, prev_end)?));

        let mut prev_total = Deliverables::default();
        let mut prev_by_project = HashMap::new();
        for digest in prev {
            prev_total.merge(&digest.deliverables);
            let key = digest
                .project
                .clone()
                .unwrap_or_else(|| "(unknown)".to_owned());
            prev_by_project.insert(key, digest.deliverables);
        }
        Ok(Some(TrendData {
            label: span.trend_label.clone(),
            prev_total,
            prev_by_project,
        }))
    }

    /// Collect (and dedup) the turns in `[start, end]` from the store plus, unless
    /// `--no-backfill`, a transcript re-scan.
    fn gather(
        &self,
        paths: &Paths,
        store: &Store,
        start: Date,
        end: Date,
    ) -> miette::Result<Vec<TurnEntry>> {
        let mut entries = store.read_range(start, end).into_diagnostic()?;
        if !self.no_backfill {
            let files = match &self.project {
                Some(cwd) => paths.session_files_for(cwd).into_diagnostic()?,
                None => paths.session_files().into_diagnostic()?,
            };
            for file in files {
                let events = read_events(&file).into_diagnostic()?;
                for entry in entries_from_events(&events) {
                    let day = local_date(entry.ts);
                    if day >= start && day <= end {
                        entries.push(entry);
                    }
                }
            }
        }
        Ok(dedup_by_uuid(entries))
    }

    /// Resolve the reporting span from the flags.
    fn resolve_span(&self) -> miette::Result<Span> {
        if let (Some(since), Some(until)) = (&self.since, &self.until) {
            let start = parse_date(since)?;
            let end = parse_date(until)?;
            if end < start {
                return Err(miette!("--until {until} is before --since {since}"));
            }
            return Ok(Span {
                start,
                end,
                label: format!("{start}_{end}"),
                title: format!("期間レポート {start}〜{end}"),
                trend_label: "前期比".to_owned(),
                monthly: false,
            });
        }
        let anchor = parse_date(&self.date)?;
        if self.month {
            let (start, end) = month_bounds(anchor)?;
            return Ok(Span {
                start,
                end,
                label: format!("{:04}-{:02}", start.year(), start.month()),
                title: format!("月報 {:04}-{:02}", start.year(), start.month()),
                trend_label: "先月比".to_owned(),
                monthly: true,
            });
        }
        let (start, end) = week_bounds(anchor)?;
        let iso = start.iso_week_date();
        Ok(Span {
            start,
            end,
            label: format!("{:04}-W{:02}", iso.year(), iso.week()),
            title: format!("週報 {start}〜{end}"),
            trend_label: "先週比".to_owned(),
            monthly: false,
        })
    }
}

/// The Monday–Sunday week containing `anchor`.
fn week_bounds(anchor: Date) -> miette::Result<(Date, Date)> {
    let offset = i64::from(anchor.weekday().to_monday_zero_offset());
    let start = anchor.checked_sub(offset.days()).into_diagnostic()?;
    let end = start.checked_add(6.days()).into_diagnostic()?;
    Ok((start, end))
}

/// The first–last day of `anchor`'s calendar month.
fn month_bounds(anchor: Date) -> miette::Result<(Date, Date)> {
    let first = Date::new(anchor.year(), anchor.month(), 1).into_diagnostic()?;
    let last = Date::new(anchor.year(), anchor.month(), first.days_in_month()).into_diagnostic()?;
    Ok((first, last))
}

/// The period immediately preceding `span` (previous month, or an equal-length
/// block ending the day before `start`).
fn previous_span(span: &Span) -> miette::Result<(Date, Date)> {
    if span.monthly {
        let prev_anchor = span.start.checked_sub(1.days()).into_diagnostic()?;
        return month_bounds(prev_anchor);
    }
    let len = (span.end - span.start).get_days() + 1;
    let prev_end = span.start.checked_sub(1.days()).into_diagnostic()?;
    let prev_start = prev_end.checked_sub((len - 1).days()).into_diagnostic()?;
    Ok((prev_start, prev_end))
}
