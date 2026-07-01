//! `completions` / `man` — generate shell completions and man pages.
//!
//! These are handled directly in `main` (before any store is opened) because they
//! need the `clap::CommandFactory` rather than a resolved context.

use std::path::{Path, PathBuf};

use clap::CommandFactory;
use clap_complete::Shell;
use miette::IntoDiagnostic as _;

/// Print a shell completion script.
#[derive(Debug, clap::Args)]
pub(crate) struct CompletionsArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub(crate) shell: Shell,
}

/// Generate man pages into a directory.
#[derive(Debug, clap::Args)]
pub(crate) struct ManArgs {
    /// Output directory.
    #[arg(value_hint = clap::ValueHint::DirPath)]
    pub(crate) out_dir: PathBuf,
}

/// Print a completion script for `shell` to stdout.
pub(crate) fn print_completions<C: CommandFactory>(shell: Shell) {
    let mut cmd = C::command();
    let name = cmd.get_name().to_owned();
    clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
}

/// Render the top-level and subcommand man pages into `out_dir`.
///
/// # Errors
/// Returns a diagnostic if the directory or any file cannot be written.
pub(crate) fn generate_man<C: CommandFactory>(out_dir: &Path) -> miette::Result<()> {
    std::fs::create_dir_all(out_dir).into_diagnostic()?;
    let cmd = C::command();
    let bin = cmd.get_name().to_owned();

    write_page(out_dir, &format!("{bin}.1"), &cmd)?;
    for sub in cmd.get_subcommands() {
        let page = format!("{bin}-{}.1", sub.get_name());
        write_page(out_dir, &page, sub)?;
    }
    Ok(())
}

/// Render one clap command to a man page file.
fn write_page(dir: &Path, file: &str, cmd: &clap::Command) -> miette::Result<()> {
    let mut buf = Vec::new();
    clap_mangen::Man::new(cmd.clone())
        .render(&mut buf)
        .into_diagnostic()?;
    std::fs::write(dir.join(file), buf).into_diagnostic()?;
    Ok(())
}
