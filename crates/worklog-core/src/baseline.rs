//! A trailing-window baseline of "normal" activity.
//!
//! Absolute thresholds ("≥12 rewrites is a lot") can't know what *this* person's
//! normal looks like. A [`Baseline`] summarizes recent activity so the attention
//! flags can fire relative to the user's own typical levels — e.g. "today's
//! hottest file was rewritten far more than your usual peak".
//!
//! Everything is integer arithmetic (totals + an active-day count) so callers can
//! derive rounded per-day averages without floating point.

use std::collections::HashMap;

use jiff::civil::Date;

use crate::digest::{TurnEntry, local_date};

/// Aggregate activity over a trailing window, as totals plus the number of days
/// that saw any activity. Per-day averages are derived on demand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baseline {
    /// Days in the window that had at least one turn.
    pub active_days: u32,
    /// Total commits across the window.
    pub commits: u32,
    /// Total merged PRs across the window.
    pub prs_merged: u32,
    /// Total turns across the window.
    pub turns: u32,
    /// Total user interrupts across the window.
    pub interruptions: u32,
    /// Sum over days of each day's single hottest-file churn.
    pub max_churn_total: u32,
}

impl Baseline {
    /// Whether the window held no activity (so flags must fall back to absolutes).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.active_days == 0
    }

    /// Rounded average commits per active day.
    #[must_use]
    pub const fn commits_per_day(&self) -> u32 {
        div_round(self.commits, self.active_days)
    }

    /// Rounded average merged PRs per active day.
    #[must_use]
    pub const fn prs_merged_per_day(&self) -> u32 {
        div_round(self.prs_merged, self.active_days)
    }

    /// Rounded average turns per active day.
    #[must_use]
    pub const fn turns_per_day(&self) -> u32 {
        div_round(self.turns, self.active_days)
    }

    /// Rounded average interrupts per active day.
    #[must_use]
    pub const fn interruptions_per_day(&self) -> u32 {
        div_round(self.interruptions, self.active_days)
    }

    /// Rounded average daily peak file-churn.
    #[must_use]
    pub const fn max_churn_per_day(&self) -> u32 {
        div_round(self.max_churn_total, self.active_days)
    }
}

/// Rounded integer division; `0` when the divisor is zero.
const fn div_round(total: u32, days: u32) -> u32 {
    match days {
        0 => 0,
        d => (total + d / 2) / d,
    }
}

/// Per-day accumulator while scanning the window.
#[derive(Default)]
struct DayAcc {
    commits: u32,
    prs_merged: u32,
    turns: u32,
    interruptions: u32,
    churn: HashMap<String, u32>,
}

/// Build a [`Baseline`] from a window's worth of turns (grouped by local day).
#[must_use]
pub fn baseline_from_entries(entries: &[TurnEntry]) -> Baseline {
    let mut days: HashMap<Date, DayAcc> = HashMap::new();
    for entry in entries {
        let acc = days.entry(local_date(entry.ts)).or_default();
        acc.commits = acc.commits.saturating_add(entry.deliverables.commits);
        acc.prs_merged = acc.prs_merged.saturating_add(entry.deliverables.prs_merged);
        acc.turns = acc.turns.saturating_add(1);
        acc.interruptions = acc.interruptions.saturating_add(entry.interruptions);
        for fc in &entry.file_churn {
            let slot = acc.churn.entry(fc.file.clone()).or_insert(0);
            *slot = slot.saturating_add(fc.count);
        }
    }

    let mut base = Baseline {
        active_days: u32::try_from(days.len()).unwrap_or(u32::MAX),
        ..Baseline::default()
    };
    for acc in days.values() {
        base.commits = base.commits.saturating_add(acc.commits);
        base.prs_merged = base.prs_merged.saturating_add(acc.prs_merged);
        base.turns = base.turns.saturating_add(acc.turns);
        base.interruptions = base.interruptions.saturating_add(acc.interruptions);
        let peak = acc.churn.values().copied().max().unwrap_or(0);
        base.max_churn_total = base.max_churn_total.saturating_add(peak);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::entries_from_events;
    use crate::transcript::parse_events;

    const TWO_DAYS: &str = concat!(
        // Day 1: 2 commits, hottest file churn 3.
        r#"{"type":"user","uuid":"u1","sessionId":"s1","cwd":"/p","timestamp":"2026-06-20T08:00:00Z","message":{"role":"user","content":"a"}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a1","sessionId":"s1","timestamp":"2026-06-20T08:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/p/x.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/p/x.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/p/x.rs"}},{"type":"tool_use","name":"Bash","input":{"command":"git commit -m a && git commit -m b"}}]}}"#,
        "\n",
        // Day 2: 0 commits, churn 1.
        r#"{"type":"user","uuid":"u2","sessionId":"s2","cwd":"/p","timestamp":"2026-06-21T08:00:00Z","message":{"role":"user","content":"b"}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a2","sessionId":"s2","timestamp":"2026-06-21T08:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/p/y.rs"}}]}}"#,
    );

    #[test]
    fn averages_over_active_days() {
        let base = baseline_from_entries(&entries_from_events(&parse_events(TWO_DAYS)));
        assert_eq!(base.active_days, 2);
        assert_eq!(base.commits, 2);
        assert_eq!(base.commits_per_day(), 1); // 2 commits / 2 days
        assert_eq!(base.turns, 2);
        // Day1 peak churn 3 + Day2 peak churn 1 = 4 → /2 = 2.
        assert_eq!(base.max_churn_total, 4);
        assert_eq!(base.max_churn_per_day(), 2);
    }

    #[test]
    fn empty_is_empty() {
        assert!(baseline_from_entries(&[]).is_empty());
    }
}
