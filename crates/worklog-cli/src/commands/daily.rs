//! `daily` — generate the daily report (nippo) for a day.
//!
//! Pulls the day's turns from the stored stream and, unless `--no-backfill`,
//! re-scans the raw transcripts so sessions that ended without a hook firing
//! still appear. The two sources are merged and deduplicated by turn id. In the
//! exec view it also builds a trailing-window **baseline** so the `要注意` flags
//! fire relative to the user's own normal rather than absolute guesses.

use std::path::PathBuf;

use jiff::ToSpan as _;
use jiff::civil::Date;
use miette::IntoDiagnostic as _;
use worklog_core::baseline::baseline_from_entries;
use worklog_core::digest::{
    aggregate, dedup_by_uuid, entries_from_events, local_date, merge_by_project,
};
use worklog_core::report::{RenderOptions, render};
use worklog_core::store::Store;
use worklog_core::summarize::NullSummarizer;
use worklog_core::transcript::read_events;
use worklog_core::{TurnEntry, memory, paths::Paths};

use crate::command::Command;
use crate::commands::{StyleArg, parse_date};

/// Generate the daily report (nippo) for a day.
#[derive(Debug, clap::Args)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these are independent CLI flags, not a state machine"
)]
pub(crate) struct DailyArgs {
    /// Day to report on: `today`, `yesterday`, or `YYYY-MM-DD`.
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

    /// Trailing days used to compute the "平常" baseline for `要注意` flags.
    #[arg(long, default_value_t = 28)]
    baseline_days: u32,

    /// Do not compute a baseline (use absolute flag thresholds).
    #[arg(long)]
    no_baseline: bool,

    /// Report style: `exec` (default; outcome-first, per project) or `detail`
    /// (per session with the prompt timeline, tool table, and files).
    #[arg(long, value_enum, default_value_t = StyleArg::Exec)]
    style: StyleArg,
}

impl Command for DailyArgs {
    fn run(self) -> miette::Result<()> {
        let date = parse_date(&self.date)?;
        let paths = Paths::discover().into_diagnostic()?;
        let store = Store::new(paths.store_dir.clone());

        let (today, base_entries) = self.collect(&paths, &store, date)?;

        let digests = aggregate(dedup_by_uuid(today));
        // The exec view rolls a project's sessions into one block; detail stays
        // per session.
        let digests = match self.style {
            StyleArg::Exec => merge_by_project(digests),
            StyleArg::Detail => digests,
        };
        let baseline = base_entries.map(|e| baseline_from_entries(&dedup_by_uuid(e)));

        let memory = memory::descriptions_by_session(&paths.projects_dir);
        let opts = RenderOptions {
            memory: &memory,
            summarizer: &NullSummarizer,
            style: self.style.into(),
            trend: None,
            baseline: baseline.as_ref(),
        };
        let markdown = render(&format!("日報 {date}"), &digests, &opts);

        if !self.no_store {
            let path = store.write_report(date, &markdown).into_diagnostic()?;
            if !self.quiet_hint() {
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

impl DailyArgs {
    /// Collect the day's turns and (for the exec view) the prior-window turns used
    /// for the baseline — in a single transcript scan.
    ///
    /// Returns `(today, Some(window))`; the window is `None` when baselining is off.
    fn collect(
        &self,
        paths: &Paths,
        store: &Store,
        date: Date,
    ) -> miette::Result<(Vec<TurnEntry>, Option<Vec<TurnEntry>>)> {
        let want_baseline =
            matches!(self.style, StyleArg::Exec) && !self.no_baseline && self.baseline_days > 0;
        let window = if want_baseline {
            let start = date
                .checked_sub(i64::from(self.baseline_days).days())
                .into_diagnostic()?;
            let prev = date.checked_sub(1.days()).into_diagnostic()?;
            Some((start, prev))
        } else {
            None
        };

        let mut today = store.read_entries(date).into_diagnostic()?;
        let mut base = match window {
            Some((start, end)) => store.read_range(start, end).into_diagnostic()?,
            None => Vec::new(),
        };

        if !self.no_backfill {
            let files = match &self.project {
                Some(cwd) => paths.session_files_for(cwd).into_diagnostic()?,
                None => paths.session_files().into_diagnostic()?,
            };
            for file in files {
                let events = read_events(&file).into_diagnostic()?;
                for entry in entries_from_events(&events) {
                    let day = local_date(entry.ts);
                    if day == date {
                        today.push(entry);
                    } else if let Some((start, end)) = window
                        && day >= start
                        && day <= end
                    {
                        base.push(entry);
                    }
                }
            }
        }
        Ok((today, window.map(|_| base)))
    }

    /// Whether to suppress the decorative "saved report" note.
    const fn quiet_hint(&self) -> bool {
        // `--out` already tells the user where it went; keep stdout clean then.
        self.out.is_some()
    }
}
