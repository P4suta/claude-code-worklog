//! worklog: turn Claude Code session logs into daily and continuous reports.
//!
//! This binary is a user-facing CLI, so writing to stdout/stderr is the whole
//! point — those print lints are relaxed here (they stay strict in the library).
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the CLI's job is to render results to the user's terminal"
)]
#![allow(
    clippy::redundant_pub_crate,
    reason = "binary crate: pub(crate) is the honest visibility, and this nursery lint conflicts with rustc's unreachable_pub"
)]

mod cli;
mod command;
mod commands;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::command::Command as _;

/// Configure tracing from `-v`/`--quiet` with `WORKLOG_LOG` as an override.
fn init_tracing(verbose: u8, quiet: bool) {
    let fallback = if quiet {
        "error"
    } else {
        match verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter = tracing_subscriber::EnvFilter::try_from_env("WORKLOG_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(fallback));
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .try_init();
}

fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);

    match cli.command {
        // Meta commands need the clap factory, not the store.
        Command::Completions(args) => {
            commands::meta::print_completions::<Cli>(args.shell);
            Ok(())
        },
        Command::Man(args) => commands::meta::generate_man::<Cli>(&args.out_dir),
        Command::Daily(args) => args.run(),
        Command::Period(args) => args.run(),
        Command::Tail(args) => args.run(),
        Command::Hook(args) => args.run(),
        Command::InstallHooks(args) => args.run(),
    }
}
