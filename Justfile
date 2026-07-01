# cc-worklog task entry points.
#
# This tool only ever reads ~/.claude data and writes to its own store, so no
# recipe needs secrets. Pure dev recipes (build/test/lint) run plainly.

default:
    @just --list

# ----- build / test -----

build:
    cargo build --workspace --all-targets

test:
    cargo test --workspace
    cargo test --doc --workspace

# ----- quality gates -----

fmt:
    cargo fmt --all
    cargo sort --workspace --grouped
    taplo fmt
    typos --write-changes

lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deny check advisories bans licenses sources
    typos
    cargo machete

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# ----- usage -----

# Generate today's daily report from local Claude Code transcripts.
daily *ARGS:
    cargo run -q -p worklog-cli -- daily {{ARGS}}

# Generate a weekly/monthly/range report with a prior-period trend.
period *ARGS:
    cargo run -q -p worklog-cli -- period {{ARGS}}

# Show today's continuous (bunpo) stream.
tail *ARGS:
    cargo run -q -p worklog-cli -- tail {{ARGS}}

# Print the settings.json hook snippet.
install-hooks:
    cargo run -q -p worklog-cli -- install-hooks

# ----- setup -----

# Install the CLI onto PATH.
install:
    cargo install --path crates/worklog-cli

# Install git hooks.
hooks:
    lefthook install
