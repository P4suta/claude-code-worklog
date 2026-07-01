//! Error type for the worklog core library.

use std::path::{Path, PathBuf};

/// Errors produced while reading transcripts or maintaining the worklog store.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A filesystem operation failed, annotated with the path it concerned.
    #[error("I/O error at {path}")]
    Io {
        /// The path the failing operation touched.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A JSON value could not be serialized or deserialized.
    #[error("JSON error")]
    Json(#[from] serde_json::Error),

    /// A timestamp or date string could not be parsed.
    #[error("could not parse {kind} {raw:?}: {message}")]
    Parse {
        /// What was being parsed (e.g. `"timestamp"`, `"date"`).
        kind: &'static str,
        /// The offending input.
        raw: String,
        /// A human-readable reason.
        message: String,
    },

    /// A required base directory (home or data dir) could not be located.
    #[error("could not locate {what}")]
    MissingDir {
        /// Which directory was missing.
        what: &'static str,
    },
}

impl CoreError {
    /// Build an [`CoreError::Io`] that remembers the `path` involved.
    #[must_use]
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

/// A `Result` whose error is [`CoreError`].
pub type Result<T> = std::result::Result<T, CoreError>;
