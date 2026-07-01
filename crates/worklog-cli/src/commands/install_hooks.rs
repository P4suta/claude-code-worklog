//! `install-hooks` — print the settings.json snippet for continuous capture.
//!
//! Writing to `settings.json` is intentionally left to the user (it is the
//! harness's configuration surface), so this command only prints a ready-to-paste
//! snippet plus guidance.

use crate::command::Command;

/// Print the settings.json hook snippet.
#[derive(Debug, clap::Args)]
pub(crate) struct InstallHooksArgs {}

/// The snippet wiring `Stop`/`SessionEnd` to this CLI.
const SNIPPET: &str = r#"{
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "worklog hook stop", "timeout": 30 } ] }
    ],
    "SessionEnd": [
      { "hooks": [ { "type": "command", "command": "worklog hook session-end", "timeout": 60 } ] }
    ]
  }
}"#;

impl Command for InstallHooksArgs {
    fn run(self) -> miette::Result<()> {
        println!("# Add the following to ~/.claude/settings.json (or a project's");
        println!("# .claude/settings.json), merging into any existing \"hooks\" block:\n");
        println!("{SNIPPET}\n");
        if let Ok(exe) = std::env::current_exe() {
            println!("# `worklog` must be on PATH. If it is not, use the absolute path:");
            println!("#   {}", exe.display());
        }
        println!("# Then `worklog daily` aggregates the captured stream into a report.");
        Ok(())
    }
}
