//! The clap command-line interface definition.

use clap::{Parser, Subcommand};

use crate::commands;

/// Worked examples appended to the top-level `--help`.
const TOP_EXAMPLES: &str = "\
Examples:
  worklog daily                      Today's daily report (nippo) to stdout
  worklog daily --date 2026-06-27    A specific day
  worklog period --week              This week's roll-up with last-week trend
  worklog period --month             This month's roll-up with last-month trend
  worklog tail                       Today's continuous stream (bunpo) as a table
  worklog install-hooks              Print the settings.json hook snippet

Run `worklog <command> --help` for command-specific options.";

/// Turn Claude Code session logs into daily and continuous work reports.
///
/// Reads transcripts under `~/.claude/projects` (read-only) and keeps its own
/// append-only stream under `~/.claude/worklog`. No network, no LLM. Override the
/// locations with `WORKLOG_CLAUDE_DIR` / `WORKLOG_STORE_DIR`.
#[derive(Debug, Parser)]
#[command(name = "worklog", version, about, long_about, after_help = TOP_EXAMPLES)]
pub(crate) struct Cli {
    /// Suppress decorative output (errors still print).
    #[arg(long, short, global = true)]
    pub(crate) quiet: bool,

    /// Increase logging verbosity (-v info, -vv debug, -vvv trace).
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    pub(crate) verbose: u8,

    #[command(subcommand)]
    pub(crate) command: Command,
}

/// The set of subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Generate the daily report (nippo) for a day.
    Daily(commands::daily::DailyArgs),
    /// Generate a weekly/monthly/range report with a prior-period trend.
    Period(commands::period::PeriodArgs),
    /// Show the continuous stream (bunpo) for a day as a table.
    Tail(commands::tail::TailArgs),
    /// Ingest a hook event from stdin (run by Claude Code's Stop/SessionEnd hooks).
    Hook(commands::hook::HookArgs),
    /// Print the settings.json hook snippet to wire up continuous capture.
    #[command(name = "install-hooks")]
    InstallHooks(commands::install_hooks::InstallHooksArgs),
    /// Print a shell completion script.
    Completions(commands::meta::CompletionsArgs),
    /// Generate man pages into a directory.
    Man(commands::meta::ManArgs),
}
