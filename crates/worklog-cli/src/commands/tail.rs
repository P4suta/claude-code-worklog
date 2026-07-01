//! `tail` — show the continuous stream (bunpo) for a day as a table.

use comfy_table::{Cell, ContentArrangement, Table};
use miette::IntoDiagnostic as _;
use worklog_core::digest::{ToolCount, TurnEntry, local_hm};
use worklog_core::paths::Paths;
use worklog_core::store::Store;

use crate::command::Command;
use crate::commands::parse_date;

/// Show the continuous stream (bunpo) for a day as a table.
#[derive(Debug, clap::Args)]
pub(crate) struct TailArgs {
    /// Day to show: `today`, `yesterday`, or `YYYY-MM-DD`.
    #[arg(long, default_value = "today")]
    date: String,
}

impl Command for TailArgs {
    fn run(self) -> miette::Result<()> {
        let date = parse_date(&self.date)?;
        let paths = Paths::discover().into_diagnostic()?;
        let store = Store::new(paths.store_dir);

        let mut entries = store.read_entries(date).into_diagnostic()?;
        if entries.is_empty() {
            println!("{date} の分報はまだありません。");
            return Ok(());
        }
        entries.sort_by_key(|e| e.ts);

        let mut table = Table::new();
        table
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec!["時刻", "プロジェクト", "リクエスト", "ツール"]);
        for entry in &entries {
            table.add_row(row_cells(entry));
        }
        println!("{table}");
        Ok(())
    }
}

/// Build the display cells for one turn.
fn row_cells(entry: &TurnEntry) -> Vec<Cell> {
    vec![
        Cell::new(local_hm(entry.ts)),
        Cell::new(entry.project.as_deref().unwrap_or("-")),
        Cell::new(entry.user_request.as_deref().unwrap_or("-")),
        Cell::new(join_tools(&entry.tools)),
    ]
}

/// Render a turn's tools as `Name×N, …`.
fn join_tools(tools: &[ToolCount]) -> String {
    if tools.is_empty() {
        return "-".to_owned();
    }
    tools
        .iter()
        .map(|t| format!("{}×{}", t.name, t.count))
        .collect::<Vec<_>>()
        .join(", ")
}
