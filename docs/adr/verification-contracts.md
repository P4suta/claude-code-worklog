---
date: 2026-10-21
---

# Select verification by Rust input contracts

## Status

Proposed.

## Context

Path handling and command execution need native tests on Linux, Windows, and macOS.
The public compiler floor is Rust 1.95.
The earlier workflows ran those tests for documentation edits, repeated checks on main, and scheduled separate audits and repository scoring.

The required status must reject a failed or unexpectedly skipped job.
Release publication must remain separate from maintenance verification.

## Decision

Run fast workflow, spelling, and applicable commit checks for each pull request and merge-group event.
Use the [path selector](../../.github/filters/ci.yml) to choose Rust lint, native tests, and compiler-floor verification.
Changes to verification entry points select the complete Rust contract.

Use the locked [Justfile commands](../../Justfile) for lint and tests.
Keep all-feature dependency assurance, strict Clippy, the three native test platforms, and the Rust 1.95.0 build.
Pin ordinary development and release builds to the tested stable compiler.
Keep the compatibility compiler tied to the declared floor.

Require explicit success for each selected job and an explicit skipped result for each unselected job.
Validate selection outputs before using them.
The [required workflow job](../../.github/workflows/ci.yml) implements the decision.

Keep dependency review on dependency and workflow inputs.
Keep source-selected CodeQL with the existing security-and-quality query suite.
Remove duplicate main verification and schedules.
Keep a manual advisory audit and manual security analysis.
Remove repository scoring, which duplicates repository configuration checks without testing the product.

Keep the existing version-tag and manual release workflow, supported targets, credentials, and artifact format.

## Consequences

Documentation-only changes avoid compilation, platform jobs, and security analysis.
Rust input changes still exercise every existing native boundary.
Future runtime inputs must be added to the selector.

Running every job for every event was rejected because it repeats unrelated work.
Allowing any skipped prerequisite through the required gate was rejected because it can hide missing verification.
Local maintenance checks grant no release authorization.
