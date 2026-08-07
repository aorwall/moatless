#!/usr/bin/env bash
# Lint a pull request's TITLE against commitlint.config.ts.
#
# Why the title, not the commits: this repo squash-merges, and GitHub composes
# the squash commit's message server-side from the PR title. No git hook runs at
# that point — .husky/commit-msg guards only local commits — and no other CI job
# runs commitlint. So the message that actually lands on main is unchecked
# unless the title is linted here, against the same commitlint.config.ts the
# local hook uses, so there is one source of truth for the rule.
#
# The title arrives in the PR_TITLE environment variable and is never
# interpolated into this script, so a title containing shell metacharacters is
# data, not code, and cannot execute.
set -euo pipefail

if [ -z "${PR_TITLE:-}" ]; then
  echo "::error::PR_TITLE is empty — this check must run on a pull_request event with the title passed via the PR_TITLE env var." >&2
  exit 1
fi

echo "Linting PR title against commitlint.config.ts:"
echo "  ${PR_TITLE}"
echo

if printf '%s\n' "$PR_TITLE" | bunx commitlint; then
  echo "PR title conforms."
else
  status=$?
  echo "::error::PR title does not conform to commitlint.config.ts. This repo squash-merges and the squash commit's message is built from this title, so it becomes the commit on main. Edit the PR title: the type must be one @commitlint/config-conventional allows, and any scope must be in the scope-enum in commitlint.config.ts." >&2
  exit "$status"
fi
