//! Locating Claude Code's data on disk and the worklog store.
//!
//! Claude Code keeps per-project transcripts under `~/.claude/projects/`, where
//! each project directory is named after its working directory with every path
//! separator and drive colon replaced by `-` (so `C:\Users\me\proj` becomes
//! `C--Users-me-proj`). The worklog store lives alongside, under
//! `~/.claude/worklog/`.
//!
//! Both locations can be redirected for tests and unusual setups:
//!
//! - `WORKLOG_CLAUDE_DIR` overrides the `~/.claude` root.
//! - `WORKLOG_STORE_DIR` overrides the store directory.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// Resolved on-disk locations the worklog tooling reads and writes.
#[derive(Debug, Clone)]
#[allow(
    clippy::struct_field_names,
    reason = "the `_dir` suffix is the meaningful, parallel naming for these paths"
)]
pub struct Paths {
    /// The Claude Code root, i.e. `~/.claude`.
    pub claude_dir: PathBuf,
    /// The transcripts root, i.e. `<claude_dir>/projects`.
    pub projects_dir: PathBuf,
    /// The worklog store root, i.e. `<claude_dir>/worklog` by default.
    pub store_dir: PathBuf,
}

impl Paths {
    /// Discover the standard locations, honoring the override environment vars.
    ///
    /// # Errors
    /// Returns [`CoreError::MissingDir`] if no `WORKLOG_CLAUDE_DIR` is set and the
    /// user's home directory cannot be determined.
    pub fn discover() -> Result<Self> {
        let claude_dir = match std::env::var_os("WORKLOG_CLAUDE_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => default_claude_dir()?,
        };
        Ok(Self::from_claude_dir(claude_dir))
    }

    /// Build paths rooted at an explicit `~/.claude` directory.
    ///
    /// The store still honors `WORKLOG_STORE_DIR`; otherwise it sits under the
    /// given root. Useful in tests with a temporary root.
    #[must_use]
    pub fn from_claude_dir(claude_dir: impl Into<PathBuf>) -> Self {
        let claude_dir = claude_dir.into();
        let projects_dir = claude_dir.join("projects");
        let store_dir = std::env::var_os("WORKLOG_STORE_DIR")
            .map_or_else(|| claude_dir.join("worklog"), PathBuf::from);
        Self {
            claude_dir,
            projects_dir,
            store_dir,
        }
    }

    /// Every top-level session transcript across all projects.
    ///
    /// Sidechain transcripts under each project's `subagents/` directory are
    /// intentionally skipped — they are tool noise, not user-facing turns.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if the projects directory cannot be read.
    pub fn session_files(&self) -> Result<Vec<PathBuf>> {
        self.session_files_filtered(None)
    }

    /// Session transcripts for a single project working directory.
    ///
    /// `cwd` is encoded the same way Claude Code names its project directories.
    ///
    /// # Errors
    /// Returns [`CoreError::Io`] if the projects directory cannot be read.
    pub fn session_files_for(&self, cwd: &Path) -> Result<Vec<PathBuf>> {
        let encoded = encode_project(cwd);
        self.session_files_filtered(Some(encoded.as_str()))
    }

    fn session_files_filtered(&self, only: Option<&str>) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        if !self.projects_dir.is_dir() {
            return Ok(files);
        }
        let dir = std::fs::read_dir(&self.projects_dir)
            .map_err(|e| CoreError::io(&self.projects_dir, e))?;
        for entry in dir {
            let entry = entry.map_err(|e| CoreError::io(&self.projects_dir, e))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(want) = only
                && path.file_name().and_then(|n| n.to_str()) != Some(want)
            {
                continue;
            }
            collect_jsonl(&path, &mut files)?;
        }
        files.sort();
        Ok(files)
    }
}

/// Collect `*.jsonl` files directly inside `dir` (not recursing into subdirs).
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let read = std::fs::read_dir(dir).map_err(|e| CoreError::io(dir, e))?;
    for entry in read {
        let entry = entry.map_err(|e| CoreError::io(dir, e))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
    Ok(())
}

/// The default `~/.claude` directory from the OS home directory.
fn default_claude_dir() -> Result<PathBuf> {
    let base = directories::BaseDirs::new().ok_or(CoreError::MissingDir {
        what: "home directory",
    })?;
    Ok(base.home_dir().join(".claude"))
}

/// Encode a working directory the way Claude Code names its project folders:
/// every `/`, `\`, and `:` becomes `-`.
#[must_use]
pub fn encode_project(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_windows_path_like_claude_code() {
        let encoded = encode_project(Path::new(r"C:\Users\me\projects"));
        assert_eq!(encoded, "C--Users-me-projects");
    }

    #[test]
    fn encodes_unix_path() {
        let encoded = encode_project(Path::new("/home/me/proj"));
        assert_eq!(encoded, "-home-me-proj");
    }

    #[test]
    fn session_files_skips_subagents_and_sorts() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_claude_dir(tmp.path());
        let proj = paths.projects_dir.join("C--proj");
        std::fs::create_dir_all(proj.join("subagents")).unwrap();
        std::fs::write(proj.join("b.jsonl"), "").unwrap();
        std::fs::write(proj.join("a.jsonl"), "").unwrap();
        std::fs::write(proj.join("notes.txt"), "").unwrap();
        std::fs::write(proj.join("subagents").join("agent.jsonl"), "").unwrap();

        let files = paths.session_files().unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["a.jsonl", "b.jsonl"]);
    }
}
