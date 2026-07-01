//! Optional back-links from Claude Code memory files.
//!
//! Each curated memory under `~/.claude/projects/<proj>/memory/*.md` carries YAML
//! frontmatter with a `description` and the `originSessionId` of the session that
//! created it. When a daily report covers that session, the description is a ready
//! one-line summary worth surfacing. Parsing is intentionally minimal — a couple
//! of `key: value` lines — to avoid pulling in a YAML dependency.

use std::collections::HashMap;
use std::path::Path;

use tracing::debug;

/// Map of `originSessionId` → memory `description`, gathered from all projects.
#[must_use]
pub fn descriptions_by_session(projects_dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(projects) = std::fs::read_dir(projects_dir) else {
        return map;
    };
    for project in projects.flatten() {
        let memory_dir = project.path().join("memory");
        collect_from_dir(&memory_dir, &mut map);
    }
    map
}

/// Read every `*.md` in a memory directory into the map.
fn collect_from_dir(memory_dir: &Path, map: &mut HashMap<String, String>) {
    let Ok(files) = std::fs::read_dir(memory_dir) else {
        return;
    };
    for file in files.flatten() {
        let path = file.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                if let Some((session, desc)) = parse_frontmatter(&text) {
                    map.insert(session, desc);
                }
            },
            Err(err) => debug!(?path, %err, "skipping unreadable memory file"),
        }
    }
}

/// Extract `(originSessionId, description)` from a file's frontmatter block.
fn parse_frontmatter(text: &str) -> Option<(String, String)> {
    let body = text.strip_prefix("---")?;
    let end = body.find("\n---")?;
    let front = &body[..end];

    let mut session = None;
    let mut description = None;
    for line in front.lines() {
        if let Some(v) = field(line, "originSessionId") {
            session = Some(v);
        } else if let Some(v) = field(line, "description") {
            description = Some(v);
        }
    }
    Some((session?, description?))
}

/// Parse a `key: value` line, returning the unquoted value for a matching key.
fn field(line: &str, key: &str) -> Option<String> {
    let rest = line.trim().strip_prefix(key)?.trim_start();
    let value = rest.strip_prefix(':')?.trim();
    let value = value.trim_matches(|c| c == '"' || c == '\'');
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_origin_and_description() {
        let md = "---\nname: foo\ndescription: A short summary\nmetadata:\n  originSessionId: abc-123\n---\n\nbody";
        let (session, desc) = parse_frontmatter(md).unwrap();
        assert_eq!(session, "abc-123");
        assert_eq!(desc, "A short summary");
    }

    #[test]
    fn missing_origin_yields_none() {
        let md = "---\ndescription: only desc\n---\nbody";
        assert!(parse_frontmatter(md).is_none());
    }

    #[test]
    fn collects_across_projects() {
        let tmp = tempfile::tempdir().unwrap();
        let mem = tmp.path().join("C--proj").join("memory");
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(
            mem.join("a.md"),
            "---\ndescription: \"Did the thing\"\nmetadata:\n  originSessionId: s-1\n---\nx",
        )
        .unwrap();
        let map = descriptions_by_session(tmp.path());
        assert_eq!(map.get("s-1").map(String::as_str), Some("Did the thing"));
    }
}
