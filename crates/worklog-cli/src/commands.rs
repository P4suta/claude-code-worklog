//! Subcommand implementations.

pub(crate) mod daily;
pub(crate) mod hook;
pub(crate) mod install_hooks;
pub(crate) mod meta;
pub(crate) mod period;
pub(crate) mod tail;

use std::str::FromStr as _;

use jiff::Zoned;
use jiff::civil::Date;
use miette::miette;
use worklog_core::report::Style;

/// Resolve a `--date` value (`today`, `yesterday`, or `YYYY-MM-DD`) to a date.
fn parse_date(spec: &str) -> miette::Result<Date> {
    match spec {
        "today" => Ok(Zoned::now().date()),
        "yesterday" => Zoned::now()
            .date()
            .yesterday()
            .map_err(|e| miette!("could not compute yesterday: {e}")),
        other => Date::from_str(other)
            .map_err(|e| miette!("invalid --date {other:?} (expected YYYY-MM-DD): {e}")),
    }
}

/// Report style (CLI mirror of [`Style`]), shared by `daily` and `period`.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub(crate) enum StyleArg {
    /// Outcome-first executive summary, grouped by project (default).
    #[default]
    Exec,
    /// Per-session detail with prompts, tool table, and files.
    Detail,
}

impl From<StyleArg> for Style {
    fn from(arg: StyleArg) -> Self {
        match arg {
            StyleArg::Exec => Self::Exec,
            StyleArg::Detail => Self::Detail,
        }
    }
}
