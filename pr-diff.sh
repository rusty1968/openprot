#!/usr/bin/env bash
# Print the diff of a GitHub PR against its merge base with origin/main.
# Usage: ./pr-diff.sh <pr-number> [base] > pr.diff
set -euo pipefail

pr="${1:?usage: $0 <pr-number> [base]}"
base="${2:-origin/main}"

git fetch origin "pull/${pr}/head"
git diff "$(git merge-base "$base" FETCH_HEAD)" FETCH_HEAD
