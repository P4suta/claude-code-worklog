# cc-worklog

Turn your Claude Code session logs into a **daily report** (日報) and a
**continuous stream** (分報) — without writing them by hand, and without an LLM.

When you build with Claude Code, you aren't the one typing, so the usual "what I
did today" write-up is awkward. But Claude Code already records every session as a
transcript. `cc-worklog` reads those transcripts and distills them into plain,
scannable Markdown built entirely from extracted facts.

The default report is an **executive summary**: grouped by project, it answers
"what was *shipped, changed, and verified*" — PRs merged (with numbers), commits,
pushes, tests, builds, and the changed areas — headlined by the curated note from
Claude Code memory. It deliberately omits prompts (intent, not outcome). Pass
`--style detail` for the per-session view with the prompt timeline and tool table.

Crucially it also surfaces the **process** — the part VCS can't show. PRs and
commits already live in GitHub; what a transcript uniquely records is *how* the
work went:

- **Hot spots / rework** — files rewritten many times (e.g. `core.rs ×43`), a
  fragility signal the final clean diff hides.
- **Where time went** — an explore / implement / verify split.
- **Friction** — user course-corrections (interrupts), reverts, force-pushes.
- **A `## 要注意` (attention) block** — the day's exceptions worth a second look
  (no-ship-yet-busy, churn spikes, high friction), so a manager scans one section.
  Flags are **baseline-relative**: a trailing window (default 28 days, `--baseline-days`)
  sets the user's "normal", and the summary's `平常比` line plus the flags fire on
  deviation from it — so a heavy churner isn't flagged for a routine day, and a
  quiet one is flagged when something spikes. `--no-baseline` falls back to absolutes.

The structured Markdown is designed to be a *complete, human-readable artifact on
its own* — and also clean input for an optional later LLM summarization pass (the
[`Summarizer`](crates/worklog-core/src/summarize.rs) seam), rather than dumping raw
logs at a model.

For longer horizons, `worklog period` rolls the same exec view up over a **week**,
**month**, or arbitrary **range**, and (unless `--no-trend`) appends a
**prior-period delta** — overall and per project — so "did we speed up or slow
down?" is answered inline (e.g. `推移(先週比): PRマージ -23 ・ test +29`). Period
reports are saved as `reports/2026-W26.md` / `reports/2026-06.md`.

- **No network, no LLM.** Pure structured extraction. (An inert
  [`Summarizer`](crates/worklog-core/src/summarize.rs) seam exists for a future
  opt-in LLM pass, but the default does nothing.)
- **Privacy-first.** Outcomes are inferred from shell commands by *classification
  only* — a `gh pr merge 9` becomes "PR #9 merged"; the raw command (a place
  secrets live) is never stored. Likewise only prompt first-lines, tool *names*,
  and file paths are kept — never assistant reasoning or raw tool output.
- **Read-only on your projects.** It only reads `~/.claude/projects` and writes to
  its own store under `~/.claude/worklog`.

## How it works

Claude Code writes a transcript per session at
`~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` (one JSON event per line).

`cc-worklog` captures work two ways, and merges them:

1. **Push (continuous, per turn).** A `Stop` hook runs `worklog hook stop` after
   each assistant turn, appending one entry to an append-only daily stream. A
   `SessionEnd` hook sweeps up anything missed. Appends are idempotent (keyed by
   turn id), so a hook that fires twice never double-counts.
2. **Pull (daily, on demand).** `worklog daily` re-scans the raw transcripts for a
   day and merges them with the stored stream — so even sessions that were killed
   before any hook fired still show up in the report.

## Concurrency

Many Claude Code instances run at once — and in automated setups, a great many.
The store is built for that:

- **One shard file per session**, spread across 256 hash buckets
  (`entries/<date>/<bb>/<session-id>.ndjson`). A session's hooks only ever append
  to *its own* shard, so concurrent instances never contend on a shared file —
  there is no lock to acquire — and even a day with an enormous number of sessions
  never piles them all into a single directory.
- **Incremental, bounded writes.** Each session keeps a cursor
  (`state/<session>.json`: byte offset + carried segmentation context). A
  `hook stop` reads only the transcript bytes appended since last time, so a long
  10-hour session never re-scans its whole (multi-MB) log per turn.
- **Reads are tolerant.** `daily`/`tail` fan in across all of a day's shards and
  skip any line that fails to parse (e.g. a partial final line caught mid-append),
  and they **dedup by turn id** — so even a pathological same-session race that
  produced a duplicate line yields a correct report.

## Install

```sh
cargo install --path crates/worklog-cli
# or, for development:
just build
```

## Usage

```sh
worklog daily                       # today's executive report (日報) to stdout + store
worklog daily --date 2026-06-27     # a specific day
worklog daily --style detail        # per-session view with prompts and tool tables
worklog period --week               # this week's roll-up + last-week trend (先週比)
worklog period --month              # this month's roll-up + last-month trend
worklog period --since A --until B  # an arbitrary range
worklog daily --project .           # limit to one project's working directory
worklog daily --out report.md       # also write the Markdown to a file
worklog tail                        # today's stream (分報) as a table
worklog install-hooks               # print the settings.json hook snippet
```

By default `daily` saves the rendered report to
`~/.claude/worklog/reports/<date>.md` and prints it. Use `--no-store` to print
only, and `--no-backfill` to use just the captured stream.

## Wiring up continuous capture (分報)

Run `worklog install-hooks` and merge its output into `~/.claude/settings.json`
(or a project's `.claude/settings.json`):

```json
{
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "worklog hook stop", "timeout": 30 } ] }
    ],
    "SessionEnd": [
      { "hooks": [ { "type": "command", "command": "worklog hook session-end", "timeout": 60 } ] }
    ]
  }
}
```

`worklog` must be on `PATH`; otherwise use the absolute path the command prints.
The hooks exit 0 and print nothing, so they never block or disrupt a conversation.

## Configuration

| Variable             | Meaning                                       | Default            |
| -------------------- | --------------------------------------------- | ------------------ |
| `WORKLOG_CLAUDE_DIR` | Override the `~/.claude` root.                | OS home `/.claude` |
| `WORKLOG_STORE_DIR`  | Override the store directory.                 | `<claude>/worklog` |
| `WORKLOG_LOG`        | `tracing` filter (e.g. `debug`).              | from `-v`          |
| `TZ`                 | Time zone used to bucket turns into days.     | system zone        |

## Development

```sh
just lint   # fmt --check + clippy -D warnings + cargo deny + typos + machete
just test   # unit + integration tests
```

## License

MIT OR Apache-2.0
