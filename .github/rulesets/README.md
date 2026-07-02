# Repository rulesets

These JSON files are the source of truth for this repo's branch/tag protection,
kept in version control so the configuration is reviewable and reproducible.
They are applied through the GitHub API, not read automatically by GitHub.

| File | Target | Effect |
| ---- | ------ | ------ |
| `main-branch.json` | default branch | PR required (0 approvals, thread resolution required), linear history, no force-push / deletion, and the `ci` + `Analyze (rust)` status checks must pass (strict / up-to-date). |
| `release-tags.json` | `refs/tags/v*` | Release tags are immutable — no deletion, no force-update. |
| `require-signed-commits.json` | all refs | Every commit must carry a verified signature. |

## Apply / update

```sh
REPO=P4suta/claude-code-worklog
for f in main-branch release-tags require-signed-commits; do
  gh api -X POST "repos/$REPO/rulesets" --input ".github/rulesets/$f.json"
done
```

To update an existing ruleset, find its id (`gh api repos/$REPO/rulesets`) and
`PUT repos/$REPO/rulesets/<id>` with the edited file.

## Notes

- `integration_id: 15368` in the status-check contexts is the GitHub Actions
  app; the `context` strings are the exact check-run names (`ci` is the
  aggregate gate job in `ci.yml`; `Analyze (rust)` is the CodeQL job).
- `bypass_actors` is empty by design — the rules apply to everyone, including
  the owner. Merges go through pull requests.
