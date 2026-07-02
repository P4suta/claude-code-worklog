# Security Policy

## Reporting a vulnerability

Please report security issues **privately** via GitHub Security Advisories:

- Go to the [Security tab](https://github.com/P4suta/claude-code-worklog/security/advisories/new)
  and open a private advisory.

Please do **not** open a public issue for security-sensitive reports.

We aim to acknowledge a report within a few days and will coordinate a fix and
disclosure timeline with you.

## Scope

`cc-worklog` runs entirely locally: it reads Claude Code transcripts under
`~/.claude/projects` (read-only) and writes to its own store under
`~/.claude/worklog`. It performs no network I/O and invokes no LLM. Reports are
built from classified facts — raw shell commands and assistant output are never
stored (see the "Privacy-first" note in the README). Findings that affect this
boundary (e.g. secrets leaking into a report, path traversal outside the store)
are in scope and especially welcome.

## Supported versions

This project is pre-1.0; only the latest release on `main` is supported.
