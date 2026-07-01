//! The uniform command interface every subcommand implements.

/// A runnable subcommand.
///
/// Each subcommand is a `clap::Args` struct that owns its parsed fields and runs
/// itself, so dispatch in `main` is one uniform match.
pub(crate) trait Command {
    /// Run the command, rendering its own output.
    fn run(self) -> miette::Result<()>;
}
