//! End-to-end CLI tests driving the real binary against a temporary
//! `~/.claude` tree. `TZ=UTC` pins local-day bucketing so dates are stable.

use std::error::Error;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

/// Boxed error so tests can use `?` instead of `unwrap` in helper code.
type TestResult = Result<(), Box<dyn Error>>;

/// One prompt and one assistant turn that edits a file, commits, merges a PR,
/// and runs tests — enough to exercise both the exec and detail views.
const TRANSCRIPT: &str = concat!(
    r#"{"type":"user","uuid":"u1","sessionId":"sess","cwd":"/work/demo","gitBranch":"main","slug":"demo-task","timestamp":"2026-06-27T12:00:00Z","message":{"role":"user","content":"build the feature"}}"#,
    "\n",
    r#"{"type":"assistant","uuid":"a1","sessionId":"sess","timestamp":"2026-06-27T12:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/work/demo/src/x.rs"}},{"type":"tool_use","name":"Bash","input":{"command":"git commit -m \"add the feature\" && gh pr merge 5 && cargo test"}}]}}"#,
    "\n",
);

/// Lay down a temp `~/.claude` with one project transcript; return its path.
fn setup(claude_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let project = claude_dir.join("projects").join("-work-demo");
    std::fs::create_dir_all(&project)?;
    let transcript = project.join("sess.jsonl");
    std::fs::write(&transcript, TRANSCRIPT)?;
    Ok(transcript)
}

/// Total non-empty lines across every `*.ndjson` shard under `dir` (recursive).
fn count_ndjson_lines(dir: &Path) -> usize {
    let mut total = 0;
    let Ok(read) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += count_ndjson_lines(&path);
        } else if path.extension().and_then(|e| e.to_str()) == Some("ndjson")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            total += text.lines().filter(|l| !l.trim().is_empty()).count();
        }
    }
    total
}

/// A `worklog` command pre-wired to the temp dirs and a UTC clock.
fn worklog(claude_dir: &Path, store_dir: &Path) -> Result<Command, Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("worklog")?;
    cmd.env("WORKLOG_CLAUDE_DIR", claude_dir)
        .env("WORKLOG_STORE_DIR", store_dir)
        .env("TZ", "UTC");
    Ok(cmd)
}

#[test]
fn daily_detail_shows_prompts_and_tools() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let claude = tmp.path().join("claude");
    let store = tmp.path().join("store");
    setup(&claude)?;

    worklog(&claude, &store)?
        .args([
            "daily",
            "--date",
            "2026-06-27",
            "--all",
            "--no-store",
            "--style",
            "detail",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# 日報 2026-06-27"))
        .stdout(predicate::str::contains("demo — demo-task"))
        .stdout(predicate::str::contains("1. build the feature"))
        .stdout(predicate::str::contains("| Edit | 1 |"))
        .stdout(predicate::str::contains("/work/demo/src/x.rs"));
    Ok(())
}

#[test]
fn daily_exec_is_default_and_shows_outcomes_not_prompts() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let claude = tmp.path().join("claude");
    let store = tmp.path().join("store");
    setup(&claude)?;

    // No --style → exec is the default.
    worklog(&claude, &store)?
        .args(["daily", "--date", "2026-06-27", "--all", "--no-store"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## サマリ"))
        .stdout(predicate::str::contains("## demo  (1セッション"))
        .stdout(predicate::str::contains("- やったこと:"))
        .stdout(predicate::str::contains("    - add the feature"))
        .stdout(predicate::str::contains(
            "出荷: PRマージ×1 (#5) ・ commit×1",
        ))
        .stdout(predicate::str::contains("検証: test×1"))
        .stdout(predicate::str::contains("変更: 1ファイル (src×1)"))
        .stdout(predicate::str::contains("build the feature").not());
    Ok(())
}

#[test]
fn hook_stop_is_idempotent_and_feeds_tail() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let claude = tmp.path().join("claude");
    let store = tmp.path().join("store");
    let transcript = setup(&claude)?;

    let payload = format!(
        r#"{{"session_id":"sess","hook_event_name":"Stop","transcript_path":"{}"}}"#,
        transcript.display().to_string().replace('\\', "/")
    );

    // Fire twice; the second fire must not duplicate the stored turn.
    for _ in 0..2 {
        worklog(&claude, &store)?
            .arg("hook")
            .arg("stop")
            .write_stdin(payload.clone())
            .assert()
            .success();
    }
    let lines = count_ndjson_lines(&store.join("entries").join("2026-06-27"));
    assert_eq!(lines, 1, "hook stop must be idempotent by turn id");

    worklog(&claude, &store)?
        .args(["tail", "--date", "2026-06-27"])
        .assert()
        .success()
        .stdout(predicate::str::contains("build the feature"))
        .stdout(predicate::str::contains("Edit×1"));
    Ok(())
}

#[test]
fn daily_empty_day_reports_nothing() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let claude = tmp.path().join("claude");
    let store = tmp.path().join("store");
    setup(&claude)?;

    worklog(&claude, &store)?
        .args(["daily", "--date", "2020-01-01", "--all", "--no-store"])
        .assert()
        .success()
        .stdout(predicate::str::contains("本日の作業ログはありません。"));
    Ok(())
}

/// A second transcript in the same project, one week earlier (1 commit, no PR).
const PRIOR_WEEK_TRANSCRIPT: &str = concat!(
    r#"{"type":"user","uuid":"p1","sessionId":"prev","cwd":"/work/demo","slug":"earlier","timestamp":"2026-06-19T12:00:00Z","message":{"role":"user","content":"earlier work"}}"#,
    "\n",
    r#"{"type":"assistant","uuid":"pa1","sessionId":"prev","timestamp":"2026-06-19T12:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"git commit -m \"earlier change\""}}]}}"#,
    "\n",
);

#[test]
fn period_range_shows_prior_trend() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let claude = tmp.path().join("claude");
    let store = tmp.path().join("store");
    setup(&claude)?; // current week: 2026-06-27 (PR merge ×1, commit ×1, test ×1)
    let project = claude.join("projects").join("-work-demo");
    std::fs::write(project.join("prev.jsonl"), PRIOR_WEEK_TRANSCRIPT)?; // prior week: commit ×1

    // Current = 06-21..06-27; prior equal-length block = 06-14..06-20 (has 06-19).
    worklog(&claude, &store)?
        .args([
            "period",
            "--since",
            "2026-06-21",
            "--until",
            "2026-06-27",
            "--all",
            "--no-store",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "# 期間レポート 2026-06-21〜2026-06-27",
        ))
        .stdout(predicate::str::contains(
            "推移(前期比): PRマージ +1 ・ test +1",
        ));
    Ok(())
}

#[test]
fn period_no_trend_omits_trend_line() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let claude = tmp.path().join("claude");
    let store = tmp.path().join("store");
    setup(&claude)?;

    worklog(&claude, &store)?
        .args([
            "period",
            "--since",
            "2026-06-21",
            "--until",
            "2026-06-27",
            "--all",
            "--no-store",
            "--no-trend",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("## サマリ"))
        .stdout(predicate::str::contains("推移(").not());
    Ok(())
}
