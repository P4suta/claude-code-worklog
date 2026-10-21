# Usage

`worklog` converts local Claude Code transcripts into daily reports, period summaries, and a continuous work stream.
Run `worklog --help` or `worklog <command> --help` for options.

## Reports

| Command | Result |
| --- | --- |
| `worklog daily` | Today's executive report |
| `worklog daily --style detail` | Per-session prompts and tool counts |
| `worklog daily --date 2026-06-27` | A selected day's report |
| `worklog period --week` | Weekly summary with a prior-period comparison |
| `worklog period --month` | Monthly summary with a prior-period comparison |
| `worklog period --since 2026-06-21 --until 2026-06-27` | A date-range report |
| `worklog tail` | The captured stream as a table |

Use `--project .` to limit a report to the current project, or `--all` for all projects.
Reports normally use local-day boundaries.
Set `TZ` when consistent bucketing across machines matters.

The default daily report groups extracted outcomes by project.
It includes classified commits, pull requests, tests, changed areas, and process signals.
The detail view includes prompt first lines and tool activity.
These are facts inferred from transcripts, not independent confirmation that a command succeeded.

Baseline-relative flags compare activity with a trailing window of 28 days by default.
Use `--baseline-days` to change that window or `--no-baseline` for fixed thresholds.
Period reports include a prior-period comparison unless `--no-trend` is set.

By default, `daily` scans transcripts, merges captured entries, saves a report, and prints it.
Use `--no-store` to print without saving the report.
Use `--no-backfill` to read only the captured stream, or `--out report.md` for an explicit report path.

## Continuous capture

`worklog install-hooks` prints the settings snippet for Claude Code's Stop and SessionEnd events.
Review it and merge it into the existing hooks in `~/.claude/settings.json` or the project's `.claude/settings.json`.
The command prints configuration; it does not install or replace settings.
Ensure `worklog` is on the hook process's `PATH`, or use an absolute executable path.

The Stop hook captures completed turns.
SessionEnd captures remaining entries.
The daily scan can backfill sessions that exited without a final hook.

## Data and configuration

| Variable | Purpose | Default |
| --- | --- | --- |
| `WORKLOG_CLAUDE_DIR` | Claude Code data directory | `~/.claude` |
| `WORKLOG_STORE_DIR` | Worklog store | `<claude-dir>/worklog` |
| `WORKLOG_LOG` | Tracing filter | Selected verbosity |
| `TZ` | Time zone for day boundaries | System time zone |

Transcripts live under `<claude-dir>/projects/<encoded-cwd>/*.jsonl`.
The tool reads that source tree and keeps its own stream, cursors, and reports in the worklog store.

| Store path | Content |
| --- | --- |
| `entries/<date>/<bucket>/<session>.ndjson` | Captured turns |
| `state/<session>.json` | Transcript cursor and carried context |
| `reports/<date>.md` | Daily reports |
| `reports/<year>-W<week>.md` | Weekly reports |
| `reports/<year>-<month>.md` | Monthly reports |

## Concurrency

Each session writes to its own shard across 256 buckets.
A cursor tracks the transcript byte offset and segmentation context.
Subsequent capture reads only appended bytes.
Readers tolerate partial JSON lines and deduplicate captured turn identifiers.

## Privacy

The default summarizer extracts structured information locally without a network or language-model call.
Command classification keeps outcomes and selected labels rather than raw shell command strings.
Captured data can still contain prompt first lines, project names, file paths, commit subjects, and memory-derived notes.
Review reports before sharing them.

The source transcript tree remains read-only.
Reports and captured state belong in the configured store.
The test suite uses synthetic transcripts and temporary stores.
