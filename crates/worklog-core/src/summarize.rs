//! The summarizer seam — present, and deliberately inert by default.
//!
//! The whole point of this tool is to build reports by **structured extraction**,
//! not by asking a model. But a future caller may want to layer an LLM pass on
//! top (e.g. behind a `--summarize` flag), so the rendering path takes a
//! [`Summarizer`]. The default [`NullSummarizer`] returns nothing, so reports stay
//! purely mechanical unless someone opts in.

use crate::digest::SessionDigest;

/// Produces an optional natural-language gloss for a session.
pub trait Summarizer {
    /// Return a one-line summary for `digest`, or `None` to add nothing.
    fn summarize(&self, digest: &SessionDigest) -> Option<String>;
}

/// The default summarizer: always returns `None`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSummarizer;

impl Summarizer for NullSummarizer {
    fn summarize(&self, _digest: &SessionDigest) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::aggregate;
    use crate::transcript::parse_events;

    #[test]
    fn null_summarizer_adds_nothing() {
        let events = parse_events(
            r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-06-27T08:00:00Z","message":{"role":"user","content":"hi"}}"#,
        );
        let digests = aggregate(crate::digest::entries_from_events(&events));
        assert!(NullSummarizer.summarize(&digests[0]).is_none());
    }
}
