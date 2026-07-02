# Contributing to cc-worklog

Thanks for your interest! This is a small, focused Rust project. The bar is
simple: keep it fast, dependency-light, and privacy-preserving.

## Development setup

Requires a stable Rust toolchain (the repo pins one via `rust-toolchain.toml`)
and [`just`](https://github.com/casey/just). Optional tooling — `lefthook`,
`typos`, `taplo`, `cargo-deny`, `cargo-machete`, `cargo-sort` — is used by the
lint recipe and git hooks.

```sh
just build            # cargo build --workspace --all-targets
just test             # unit + integration + doc tests
just lint             # fmt --check + clippy -D warnings + deny + typos + machete
just fmt              # auto-format (rustfmt + taplo + cargo-sort + typos)
just hooks            # install the lefthook git hooks (runs the above pre-commit/push)
```

Please run `just lint` and `just test` before opening a PR — CI runs the same
checks across Linux, Windows, and macOS, plus an MSRV build.

## Guidelines

- **Match the surrounding code.** Naming, comment density, and idioms should be
  indistinguishable from what's already there.
- **Keep the privacy invariants.** Never store raw shell commands, assistant
  reasoning, or raw tool output — only classified outcomes, prompt first-lines,
  tool names, and file paths. See the README's "Privacy-first" section.
- **No network, no LLM in the default path.** The optional `Summarizer` seam
  (`crates/worklog-core/src/summarize.rs`) exists for a future opt-in pass and
  must stay inert by default.
- **Commits** follow [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat:`, `fix:`, `docs:`, `ci:`, `refactor:`, …). Keep the subject ≤ 50 chars.
- **Add tests** for behavior changes; extraction/classification logic is heavily
  unit-tested (`crates/worklog-core/src/*`).

## Releasing (maintainers)

Push a `vX.Y.Z` tag — `release.yml` builds the cross-platform binaries and
publishes a draft GitHub Release with a `SHA256SUMS.txt` and a Sigstore
build-provenance attestation. Update `CHANGELOG.md` in the same change.

## License

By contributing you agree that your contributions are dual-licensed under
MIT OR Apache-2.0, matching the project.
