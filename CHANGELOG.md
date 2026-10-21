# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cross-platform tests, Rust 1.95 compatibility checks, dependency assurance, and change-selected security analysis.
- Tag-driven release workflow producing signed-provenance binary archives for four targets, each bundling shell completions and man pages.
- Dependabot (cargo + github-actions), community docs, and issue/PR templates.

### Changed

- Update table rendering to comfy-table 8 while keeping CLI commands and stored data formats.
- Pin the development compiler and select verification by affected inputs.
- Replace repeated and scheduled verification with pull request checks and manual audits.
- Provide a concise generated README and a separate operating guide.

## [0.1.0] - 2026-07-02

### Added

- Initial public release. `worklog daily` / `period` / `tail` build executive
  and detailed reports (日報・分報) from Claude Code transcripts via pure
  structured extraction — no network, no LLM.
- Continuous capture through `worklog hook stop` / `session-end`, a sharded
  concurrent store, and baseline-relative attention flags.

[Unreleased]: https://github.com/P4suta/claude-code-worklog/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/P4suta/claude-code-worklog/releases/tag/v0.1.0
