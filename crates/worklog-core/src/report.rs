//! Rendering a day's [`SessionDigest`]s to a Markdown daily report (nippo).
//!
//! Two styles, both built entirely from extracted facts — no model in the loop:
//!
//! - [`Style::Exec`] (default): an outcome-first executive summary, grouped by
//!   project. Each block answers "what was shipped / changed / verified", with the
//!   curated memory back-link as the headline. User prompts are intentionally
//!   omitted — they are intent, not outcome.
//! - [`Style::Detail`]: the per-session view with the prompt timeline, tool table,
//!   and touched files.

use std::collections::HashMap;
use std::fmt::Write as _;

use jiff::Timestamp;

use crate::baseline::Baseline;
use crate::digest::{
    Deliverables, EffortMix, SessionDigest, ToolCount, effort_mix, file_basename, hotspots,
    local_hm, tool_totals, top_areas,
};
use crate::summarize::Summarizer;

/// Which report to render.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Style {
    /// Outcome-first executive summary, grouped by project (default).
    #[default]
    Exec,
    /// Per-session detail with prompts, tool table, and files.
    Detail,
}

/// Enrichment and style passed to [`render`].
pub struct RenderOptions<'a> {
    /// `originSessionId` → memory description, for back-links.
    pub memory: &'a HashMap<String, String>,
    /// The summarizer to consult per block (default no-op).
    pub summarizer: &'a dyn Summarizer,
    /// Which report to render.
    pub style: Style,
    /// Prior-period comparison for trend lines (exec only; `None` for daily).
    pub trend: Option<&'a TrendData>,
    /// Trailing-window baseline for "平常比" flags (exec daily; `None` disables).
    pub baseline: Option<&'a Baseline>,
}

/// A prior period's totals, for "前期比" trend lines.
pub struct TrendData {
    /// How to label the comparison, e.g. `"先週比"` / `"先月比"`.
    pub label: String,
    /// Deliverables across the whole prior period.
    pub prev_total: Deliverables,
    /// Prior-period deliverables keyed by project name.
    pub prev_by_project: HashMap<String, Deliverables>,
}

/// How many tools to list in the detail-view summary line.
const SUMMARY_TOOL_LIMIT: usize = 6;
/// How many changed areas to name per block.
const TOP_AREAS: usize = 3;

/// Render a Markdown report titled `title` from `digests`.
///
/// In [`Style::Exec`] the digests are expected to be project-merged
/// (`digest::merge_by_project`); in [`Style::Detail`] they are per-session.
#[must_use]
pub fn render(title: &str, digests: &[SessionDigest], opts: &RenderOptions<'_>) -> String {
    let mut out = String::new();
    // Writing to a String is infallible; discard the formatter Result once.
    let _ = render_into(&mut out, title, digests, opts);
    out
}

/// The fallible inner renderer (every `write!` returns `fmt::Result`).
fn render_into(
    out: &mut String,
    title: &str,
    digests: &[SessionDigest],
    opts: &RenderOptions<'_>,
) -> std::fmt::Result {
    writeln!(out, "# {title}\n")?;
    if digests.is_empty() {
        writeln!(out, "本日の作業ログはありません。")?;
        return Ok(());
    }
    match opts.style {
        Style::Exec => render_exec(out, digests, opts)?,
        Style::Detail => render_detail(out, digests, opts)?,
    }
    Ok(())
}

// ----- executive style -----

/// Render the outcome-first executive summary.
fn render_exec(
    out: &mut String,
    digests: &[SessionDigest],
    opts: &RenderOptions<'_>,
) -> std::fmt::Result {
    let totals = sum_deliverables(digests);
    let sessions: u32 = digests.iter().map(|d| d.session_count).sum();

    writeln!(out, "## サマリ\n")?;
    if let (Some(start), Some(end)) = (min_start(digests), max_end(digests)) {
        writeln!(
            out,
            "- 稼働: {}プロジェクト ・ {sessions}セッション ・ {}–{}",
            digests.len(),
            local_hm(start),
            local_hm(end)
        )?;
    }
    if let Some(line) = shipped_line(&totals, false) {
        writeln!(out, "- 出荷: {line}")?;
    }
    if let Some(line) = verified_line(&totals) {
        writeln!(out, "- 検証: {line}")?;
    }
    if let Some(line) = risk_line(&totals) {
        writeln!(out, "- ⚠ 注意: {line}")?;
    }
    if let Some(trend) = opts.trend {
        writeln!(
            out,
            "- 推移({}): {}",
            trend.label,
            trend_line(&totals, &trend.prev_total)
        )?;
    }
    if let Some(base) = opts.baseline.filter(|b| !b.is_empty()) {
        writeln!(out, "- 平常比: {}", normal_line(&totals, base))?;
    }
    writeln!(
        out,
        "\n_Claude Code の作業ログから自動生成（LLM 不使用）。_\n"
    )?;

    let flags = attention_flags(digests, opts.baseline.filter(|b| !b.is_empty()));
    if !flags.is_empty() {
        writeln!(out, "## 要注意\n")?;
        for flag in &flags {
            writeln!(out, "- {flag}")?;
        }
        out.push('\n');
    }

    for digest in digests {
        render_exec_block(out, digest, opts)?;
    }
    Ok(())
}

// Absolute fallbacks, used when there is no baseline (no history yet).
/// Turn count above which a no-ship day reads as "lots of effort, nothing landed".
const NO_SHIP_TURNS: u32 = 5;
/// Churn count that flags a file as unusually unstable (no-baseline default).
const CHURN_SPIKE: u32 = 12;
/// Interrupts above which a project reads as high-friction.
const HIGH_INTERRUPTS: u32 = 3;

/// How many times a metric must exceed its baseline average to be "unusual".
const SPIKE_FACTOR: u32 = 2;
/// Churn below this is too trivial to flag even for a very quiet baseline.
const CHURN_FLOOR: u32 = 6;

/// Heuristic "what deserves a second look" flags across the day's projects.
///
/// When a `baseline` is supplied the thresholds are relative to the user's own
/// recent norm (e.g. "far more rewrites than your usual peak"); otherwise they
/// fall back to absolute defaults.
fn attention_flags(digests: &[SessionDigest], baseline: Option<&Baseline>) -> Vec<String> {
    let churn_min = baseline.map_or(CHURN_SPIKE, |b| {
        (b.max_churn_per_day() * SPIKE_FACTOR).max(CHURN_FLOOR)
    });
    let interrupt_min = baseline.map_or(HIGH_INTERRUPTS, |b| {
        (b.interruptions_per_day() * SPIKE_FACTOR).max(HIGH_INTERRUPTS)
    });
    let noship_min = baseline.map_or(NO_SHIP_TURNS, |b| b.turns_per_day().max(NO_SHIP_TURNS));
    let vs = if baseline.is_some() { "平常比" } else { "" };

    let mut flags = Vec::new();
    for d in digests {
        let project = d.project.as_deref().unwrap_or("(unknown)");
        let shipped = d.deliverables.prs_merged + d.deliverables.commits;
        if shipped == 0 && d.turn_count >= noship_min {
            flags.push(format!(
                "{project}: 出荷なし（{}ターン稼働、探索/難航の疑い）",
                d.turn_count
            ));
        }
        if let Some(hot) = hotspots(d, churn_min, 1).first() {
            flags.push(format!(
                "{project}: {} を {}回書き直し（{vs}多く、設計が不安定な可能性）",
                file_basename(&hot.file),
                hot.count
            ));
        }
        if d.interruptions >= interrupt_min {
            flags.push(format!(
                "{project}: 軌道修正 {}回（{vs}多く、要件が曖昧 or 難航）",
                d.interruptions
            ));
        }
        if d.deliverables.reverts > 0 || d.deliverables.force_pushes > 0 {
            flags.push(format!("{project}: revert/force-push あり（巻き戻し発生）"));
        }
    }
    flags
}

/// The "平常比" summary line: today's ship volume against the baseline daily average.
fn normal_line(today: &Deliverables, base: &Baseline) -> String {
    let ship = today.prs_merged + today.commits;
    let avg = base.prs_merged_per_day() + base.commits_per_day();
    if avg == 0 {
        return format!("出荷 {ship}件（平常 0/日, 直近{}日比）", base.active_days);
    }
    let pct = i64::from(ship) * 100 / i64::from(avg) - 100;
    format!(
        "出荷 {ship}件（平常 {avg}/日, {pct:+}% ・ 直近{}日比）",
        base.active_days
    )
}

/// Render one project's executive block.
fn render_exec_block(
    out: &mut String,
    digest: &SessionDigest,
    opts: &RenderOptions<'_>,
) -> std::fmt::Result {
    let project = digest.project.as_deref().unwrap_or("(unknown)");
    writeln!(
        out,
        "## {project}  ({}セッション, {}–{})\n",
        digest.session_count,
        local_hm(digest.start),
        local_hm(digest.end)
    )?;

    for note in memory_notes(digest, opts.memory) {
        writeln!(out, "- 成果: {note}")?;
    }
    if let Some(summary) = opts.summarizer.summarize(digest) {
        writeln!(out, "- 要約: {summary}")?;
    }
    render_highlights(out, &digest.highlights)?;
    if let Some(line) = shipped_line(&digest.deliverables, true) {
        writeln!(out, "- 出荷: {line}")?;
    }
    if !digest.files_touched.is_empty() {
        writeln!(out, "- 変更: {}", changed_line(digest))?;
    }
    if let Some(line) = verified_line(&digest.deliverables) {
        writeln!(out, "- 検証: {line}")?;
    }
    render_process(out, digest)?;
    if let Some(line) = risk_line(&digest.deliverables) {
        writeln!(out, "- ⚠ 注意: {line}")?;
    }
    if let Some(trend) = opts.trend
        && let Some(prev) = trend.prev_by_project.get(project)
        && let Some(line) = project_trend_line(&digest.deliverables, prev)
    {
        writeln!(out, "- 推移: {line}")?;
    }
    out.push('\n');
    Ok(())
}

/// The signed delta `cur - prev`.
fn delta(cur: u32, prev: u32) -> i64 {
    i64::from(cur) - i64::from(prev)
}

/// Format a metric delta as `name ±N`, or `None` when unchanged.
fn delta_part(name: &str, cur: u32, prev: u32) -> Option<String> {
    let d = delta(cur, prev);
    if d == 0 {
        None
    } else {
        Some(format!("{name} {d:+}"))
    }
}

/// The whole-report trend line (always shown; "横ばい" when nothing moved, and a
/// note when there is no prior period at all).
fn trend_line(cur: &Deliverables, prev: &Deliverables) -> String {
    if prev.is_empty() {
        return "前期間データなし".to_owned();
    }
    let parts: Vec<String> = [
        delta_part("PRマージ", cur.prs_merged, prev.prs_merged),
        delta_part("commit", cur.commits, prev.commits),
        delta_part("push", cur.pushes, prev.pushes),
        delta_part("test", cur.tests, prev.tests),
        delta_part("build", cur.builds, prev.builds),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        "横ばい".to_owned()
    } else {
        parts.join(" ・ ")
    }
}

/// A per-project trend line, or `None` when nothing changed.
fn project_trend_line(cur: &Deliverables, prev: &Deliverables) -> Option<String> {
    let parts: Vec<String> = [
        delta_part("PRマージ", cur.prs_merged, prev.prs_merged),
        delta_part("commit", cur.commits, prev.commits),
        delta_part("test", cur.tests, prev.tests),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" ・ "))
    }
}

/// Min mutating edits for a file to count as a churn "hot spot".
const HOTSPOT_MIN: u32 = 3;
/// How many hot spots to name per block.
const HOTSPOT_LIMIT: usize = 3;

/// Render the process lenses unique to Claude Code logs (not in VCS): where time
/// went, what was rewritten repeatedly, and how much the user had to step in.
fn render_process(out: &mut String, digest: &SessionDigest) -> std::fmt::Result {
    if let Some(line) = effort_line(effort_mix(digest)) {
        writeln!(out, "- 時間の使い道: {line}")?;
    }
    let spots = hotspots(digest, HOTSPOT_MIN, HOTSPOT_LIMIT);
    if !spots.is_empty() {
        let list = spots
            .iter()
            .map(|c| format!("{}×{}", file_basename(&c.file), c.count))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "- 手戻り(ホットスポット): {list}")?;
    }
    if digest.interruptions > 0 {
        writeln!(out, "- 介入: 中断×{}", digest.interruptions)?;
    }
    Ok(())
}

/// `探索X% 実装Y% 検証Z%`, or `None` when there was no measurable activity.
fn effort_line(mix: EffortMix) -> Option<String> {
    let total = mix.total();
    if total == 0 {
        return None;
    }
    let pct = |n: u32| u64::from(n) * 100 / u64::from(total);
    Some(format!(
        "探索{}% 実装{}% 検証{}%",
        pct(mix.explore),
        pct(mix.implement),
        pct(mix.verify)
    ))
}

/// How many work-item highlights to list per block before summarizing the rest.
const HIGHLIGHT_LIMIT: usize = 8;

/// Render the "やったこと" list of commit subjects and PR titles.
fn render_highlights(out: &mut String, highlights: &[String]) -> std::fmt::Result {
    if highlights.is_empty() {
        return Ok(());
    }
    writeln!(out, "- やったこと:")?;
    for item in highlights.iter().take(HIGHLIGHT_LIMIT) {
        writeln!(out, "    - {item}")?;
    }
    let extra = highlights.len().saturating_sub(HIGHLIGHT_LIMIT);
    if extra > 0 {
        writeln!(out, "    - …他 {extra} 件")?;
    }
    Ok(())
}

/// `- ⚠ 注意:` content (reverts / force pushes), or `None` if clean.
fn risk_line(d: &Deliverables) -> Option<String> {
    let mut parts = Vec::new();
    if d.reverts > 0 {
        parts.push(format!("revert×{}", d.reverts));
    }
    if d.force_pushes > 0 {
        parts.push(format!("force-push×{}", d.force_pushes));
    }
    join_parts(&parts)
}

/// The memory back-links for a block's member sessions (distinct, in order).
fn memory_notes(digest: &SessionDigest, memory: &HashMap<String, String>) -> Vec<String> {
    let mut notes = Vec::new();
    for session in &digest.members {
        if let Some(desc) = memory.get(session)
            && !notes.contains(desc)
        {
            notes.push(desc.clone());
        }
    }
    notes
}

/// `- 変更:` content: file count plus the top changed areas.
fn changed_line(digest: &SessionDigest) -> String {
    let areas = top_areas(&digest.files_touched, digest.cwd.as_deref(), TOP_AREAS);
    let detail = areas
        .iter()
        .map(|(area, count)| format!("{area}×{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    if detail.is_empty() {
        format!("{}ファイル", digest.files_touched.len())
    } else {
        format!("{}ファイル ({detail})", digest.files_touched.len())
    }
}

/// `- 出荷:` content, or `None` if nothing shipped. `with_refs` appends the
/// merged PR numbers (used per project, omitted in the cross-project summary).
fn shipped_line(d: &Deliverables, with_refs: bool) -> Option<String> {
    let mut parts = Vec::new();
    if d.prs_merged > 0 {
        let refs = if with_refs {
            pr_ref_suffix(&d.pr_refs)
        } else {
            String::new()
        };
        parts.push(format!("PRマージ×{}{refs}", d.prs_merged));
    }
    if d.prs_created > 0 {
        parts.push(format!("PR作成×{}", d.prs_created));
    }
    if d.commits > 0 {
        parts.push(format!("commit×{}", d.commits));
    }
    if d.pushes > 0 {
        parts.push(format!("push×{}", d.pushes));
    }
    if d.releases > 0 {
        parts.push(format!("release×{}", d.releases));
    }
    join_parts(&parts)
}

/// `- 検証:` content, or `None` if nothing was tested or built.
fn verified_line(d: &Deliverables) -> Option<String> {
    let mut parts = Vec::new();
    if d.tests > 0 {
        parts.push(format!("test×{}", d.tests));
    }
    if d.builds > 0 {
        parts.push(format!("build×{}", d.builds));
    }
    join_parts(&parts)
}

/// ` (#7 #12)`-style suffix listing PR numbers, or empty.
fn pr_ref_suffix(refs: &[u32]) -> String {
    if refs.is_empty() {
        return String::new();
    }
    let list = refs
        .iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(" ({list})")
}

/// Join non-empty parts with ` ・ `, or `None` if there are none.
fn join_parts(parts: &[String]) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" ・ "))
    }
}

/// Sum deliverables across every digest.
fn sum_deliverables(digests: &[SessionDigest]) -> Deliverables {
    let mut total = Deliverables::default();
    for digest in digests {
        total.merge(&digest.deliverables);
    }
    total
}

/// The earliest block start, if any.
fn min_start(digests: &[SessionDigest]) -> Option<Timestamp> {
    digests.iter().map(|d| d.start).min()
}

/// The latest block end, if any.
fn max_end(digests: &[SessionDigest]) -> Option<Timestamp> {
    digests.iter().map(|d| d.end).max()
}

// ----- detail style -----

/// Render the per-session detailed view (prompts, tools, files).
fn render_detail(
    out: &mut String,
    digests: &[SessionDigest],
    opts: &RenderOptions<'_>,
) -> std::fmt::Result {
    let turns: u32 = digests.iter().map(|d| d.turn_count).sum();
    writeln!(out, "- セッション数: {} / ターン数: {turns}", digests.len())?;
    let totals = tool_totals(digests);
    if !totals.is_empty() {
        writeln!(
            out,
            "- 主要ツール: {}",
            join_tools(&totals, SUMMARY_TOOL_LIMIT)
        )?;
    }
    writeln!(
        out,
        "\n_Claude Code の作業ログから自動生成（LLM 不使用）。_\n"
    )?;

    for digest in digests {
        render_session(out, digest, opts)?;
    }
    Ok(())
}

/// Render one session block (detail view).
fn render_session(
    out: &mut String,
    digest: &SessionDigest,
    opts: &RenderOptions<'_>,
) -> std::fmt::Result {
    let project = digest.project.as_deref().unwrap_or("(unknown)");
    let topic = digest
        .slug
        .as_deref()
        .map_or_else(|| short_id(&digest.session_id), ToOwned::to_owned);
    writeln!(out, "## {project} — {topic}\n")?;
    writeln!(
        out,
        "- 時間: {}–{}",
        local_hm(digest.start),
        local_hm(digest.end)
    )?;
    match digest.cwd.as_deref() {
        Some(cwd) => writeln!(out, "- プロジェクト: {project} (`{cwd}`)")?,
        None => writeln!(out, "- プロジェクト: {project}")?,
    }
    if let Some(branch) = &digest.git_branch {
        writeln!(out, "- ブランチ: {branch}")?;
    }
    writeln!(out, "- セッション: {}", digest.session_id)?;
    if let Some(desc) = opts.memory.get(&digest.session_id) {
        writeln!(out, "- 関連メモ: {desc}")?;
    }
    out.push('\n');

    render_requests(out, &digest.requests)?;
    render_tools(out, &digest.tools)?;
    render_files(out, &digest.files_touched)?;
    Ok(())
}

/// Render the numbered request timeline.
fn render_requests(out: &mut String, requests: &[String]) -> std::fmt::Result {
    if requests.is_empty() {
        return Ok(());
    }
    writeln!(out, "### リクエスト\n")?;
    for (i, request) in requests.iter().enumerate() {
        writeln!(out, "{}. {request}", i + 1)?;
    }
    out.push('\n');
    Ok(())
}

/// Render the tool-usage table.
fn render_tools(out: &mut String, tools: &[ToolCount]) -> std::fmt::Result {
    if tools.is_empty() {
        return Ok(());
    }
    writeln!(out, "### ツール使用\n")?;
    writeln!(out, "| ツール | 回数 |")?;
    writeln!(out, "| --- | ---: |")?;
    for tool in tools {
        writeln!(out, "| {} | {} |", tool.name, tool.count)?;
    }
    out.push('\n');
    Ok(())
}

/// Render the touched-files list.
fn render_files(out: &mut String, files: &[String]) -> std::fmt::Result {
    if files.is_empty() {
        return Ok(());
    }
    writeln!(out, "### 触れたファイル\n")?;
    for file in files {
        writeln!(out, "- {file}")?;
    }
    out.push('\n');
    Ok(())
}

/// The first segment of a session id, for compact display.
fn short_id(session_id: &str) -> String {
    session_id
        .split('-')
        .next()
        .unwrap_or(session_id)
        .to_owned()
}

/// Join the top `limit` tools as `Name×N, …`.
fn join_tools(tools: &[ToolCount], limit: usize) -> String {
    tools
        .iter()
        .take(limit)
        .map(|t| format!("{}×{}", t.name, t.count))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::{aggregate, entries_from_events, merge_by_project};
    use crate::summarize::NullSummarizer;
    use crate::transcript::parse_events;

    const SAMPLE: &str = concat!(
        r#"{"type":"user","uuid":"u1","sessionId":"s1-aaaa","cwd":"/home/me/proj","gitBranch":"main","slug":"do-stuff","timestamp":"2026-06-27T08:00:00Z","message":{"role":"user","content":"first task"}}"#,
        "\n",
        r#"{"type":"assistant","uuid":"a1","sessionId":"s1-aaaa","timestamp":"2026-06-27T08:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/home/me/proj/src/x.rs"}},{"type":"tool_use","name":"Bash","input":{"command":"git commit -m \"fix: the bug\" && gh pr merge 5 && cargo test"}}]}}"#,
    );

    fn opts(memory: &HashMap<String, String>, style: Style) -> RenderOptions<'_> {
        RenderOptions {
            memory,
            summarizer: &NullSummarizer,
            style,
            trend: None,
            baseline: None,
        }
    }

    #[test]
    fn exec_shows_outcomes_not_prompts() {
        let digests = merge_by_project(aggregate(entries_from_events(&parse_events(SAMPLE))));
        let memory = HashMap::new();
        let md = render("日報 2026-06-27", &digests, &opts(&memory, Style::Exec));

        assert!(md.contains("## サマリ"));
        assert!(md.contains("## proj  (1セッション"));
        assert!(md.contains("- やったこと:"));
        assert!(md.contains("    - fix: the bug"));
        assert!(md.contains("出荷: PRマージ×1 (#5) ・ commit×1"));
        assert!(md.contains("検証: test×1"));
        assert!(md.contains("変更: 1ファイル (src×1)"));
        // The prompt text must NOT appear in the exec view.
        assert!(!md.contains("first task"));
    }

    #[test]
    fn exec_uses_memory_backlink_after_project_merge() {
        let digests = merge_by_project(aggregate(entries_from_events(&parse_events(SAMPLE))));
        let mut memory = HashMap::new();
        memory.insert("s1-aaaa".to_owned(), "Shipped PR #5".to_owned());
        let md = render("日報 2026-06-27", &digests, &opts(&memory, Style::Exec));
        assert!(md.contains("- 成果: Shipped PR #5"));
    }

    #[test]
    fn detail_shows_prompts() {
        let digests = aggregate(entries_from_events(&parse_events(SAMPLE)));
        let memory = HashMap::new();
        let md = render("日報 2026-06-27", &digests, &opts(&memory, Style::Detail));
        assert!(md.contains("## proj — do-stuff"));
        assert!(md.contains("1. first task"));
        assert!(md.contains("| Edit | 1 |"));
    }

    #[test]
    fn empty_day_has_placeholder() {
        let memory = HashMap::new();
        let md = render("日報 2026-06-27", &[], &opts(&memory, Style::Exec));
        assert!(md.contains("本日の作業ログはありません。"));
    }

    #[test]
    fn exec_renders_trend_line() {
        let digests = merge_by_project(aggregate(entries_from_events(&parse_events(SAMPLE))));
        let memory = HashMap::new();
        // Prior period: 0 merges, 3 commits → expect PRマージ +1, commit -2.
        let prev = Deliverables {
            commits: 3,
            ..Deliverables::default()
        };
        let mut prev_by_project = HashMap::new();
        prev_by_project.insert("proj".to_owned(), prev.clone());
        let trend = TrendData {
            label: "先週比".to_owned(),
            prev_total: prev,
            prev_by_project,
        };
        let options = RenderOptions {
            memory: &memory,
            summarizer: &NullSummarizer,
            style: Style::Exec,
            trend: Some(&trend),
            baseline: None,
        };
        let md = render("週報", &digests, &options);
        assert!(md.contains("- 推移(先週比): PRマージ +1 ・ commit -2 ・ test +1"));
        // Per-project trend line too.
        assert!(md.contains("- 推移: PRマージ +1 ・ commit -2 ・ test +1"));
    }

    #[test]
    fn trend_line_handles_empty_and_flat() {
        let cur = Deliverables {
            commits: 2,
            ..Deliverables::default()
        };
        assert_eq!(
            trend_line(&cur, &Deliverables::default()),
            "前期間データなし"
        );
        assert_eq!(trend_line(&cur, &cur), "横ばい");
    }

    #[test]
    fn exec_shows_normal_comparison_against_baseline() {
        let digests = merge_by_project(aggregate(entries_from_events(&parse_events(SAMPLE))));
        let memory = HashMap::new();
        // Baseline: 4 active days, 4 commits (→ 1/day). Today ships 2 (commit+PR) → +100%.
        let base = Baseline {
            active_days: 4,
            commits: 4,
            turns: 8,
            ..Baseline::default()
        };
        let options = RenderOptions {
            memory: &memory,
            summarizer: &NullSummarizer,
            style: Style::Exec,
            trend: None,
            baseline: Some(&base),
        };
        let md = render("日報", &digests, &options);
        assert!(md.contains("- 平常比: 出荷 2件（平常 1/日, +100% ・ 直近4日比）"));
    }

    #[test]
    fn churn_flag_is_relative_to_baseline() {
        // One file edited 6 times.
        let jsonl = concat!(
            r#"{"type":"user","uuid":"u1","sessionId":"s1","cwd":"/p","timestamp":"2026-06-27T08:00:00Z","message":{"role":"user","content":"x"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","sessionId":"s1","timestamp":"2026-06-27T08:01:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/p/hot.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/p/hot.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/p/hot.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/p/hot.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/p/hot.rs"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/p/hot.rs"}}]}}"#,
        );
        let digests = merge_by_project(aggregate(entries_from_events(&parse_events(jsonl))));

        // Quiet baseline (peak churn ~1/day): 6 rewrites is unusual → flagged.
        let quiet = Baseline {
            active_days: 4,
            max_churn_total: 4,
            turns: 8,
            ..Baseline::default()
        };
        let flags = attention_flags(&digests, Some(&quiet));
        assert!(
            flags
                .iter()
                .any(|f| f.contains("hot.rs") && f.contains("平常比"))
        );

        // Busy baseline (peak churn ~20/day): 6 is normal → not flagged.
        let busy = Baseline {
            active_days: 4,
            max_churn_total: 80,
            turns: 8,
            ..Baseline::default()
        };
        let flags = attention_flags(&digests, Some(&busy));
        assert!(!flags.iter().any(|f| f.contains("hot.rs")));
    }
}
