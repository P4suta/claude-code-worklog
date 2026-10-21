# Contributing to cc-worklog

Use Rust 1.99.1 and just 1.59.0.
The workspace's supported compiler floor remains Rust 1.95.
The repository pins the development toolchain in `rust-toolchain.toml`.

## Checks

```sh
just build
just lint
just test
```

Linting uses rustfmt, strict Clippy, cargo-deny, typos, and cargo-machete.
CI pins cargo-deny 0.20.2, cargo-machete 0.9.2, and typos 1.51.1.
The checked commands use the committed lockfile.
The dependency check covers all features, advisories, licenses, bans, and sources.

Native tests run on Linux, Windows, and macOS for affected Rust inputs.
A separate Linux job checks all targets with Rust 1.95.0.
Documentation changes keep fast workflow, spelling, and commit checks.
The [verification decision](docs/adr/verification-contracts.md) describes that selection.

`just fmt` applies the existing formatters.
`just hooks` installs the project's hooks when no shared dispatcher already manages them.

## Changes

Keep changes small and match surrounding code.
Add meaningful tests for changed behavior.
Keep the existing stored-data formats and [privacy behavior](docs/usage.md#privacy).
Keep the default summarizer local and deterministic.

Use Conventional Commits and keep the subject within 50 characters.
Update the changelog for user-visible changes.
Keep the README concise and operating details in [the usage guide](docs/usage.md).

Dependency updates inherit the shared Renovate preset.
The Rust 1.95.0 compiler is a compatibility target, so its update constraint changes with the declared compiler floor.
Normal development and release compiler pins update independently.

## Releases and license

The version-tag workflow builds the existing four native targets and creates draft release assets with checksums and provenance.
Maintenance verification has no release publication authority.

Contributions keep the project's existing dual license, MIT OR Apache-2.0.
