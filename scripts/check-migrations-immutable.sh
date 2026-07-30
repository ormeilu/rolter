#!/usr/bin/env bash
# migrations are append-only: sqlx records a checksum per applied migration and
# refuses to start when a file it already ran no longer hashes the same
#
#     Error: store error: migration 18 was previously applied but has been modified
#
# a deployment hits that on the *next* upgrade, long after the change merged, so
# CI has to catch it here. editing a shipped migration is never the fix — add a
# new one. even a whitespace-only edit breaks the checksum, which is how #710
# broke 0018 (an end-of-file-fixer hook stripped a trailing newline).
set -euo pipefail

base="${1:-origin/master}"
dir="crates/rolter-store/migrations"

if ! git rev-parse --verify --quiet "$base" >/dev/null; then
    echo "check-migrations-immutable: no such ref '$base'; skipping" >&2
    exit 0
fi

# fork point, so the diff shows only what this branch did to the directory and
# never what master added after it branched
merge_base=$(git merge-base "$base" HEAD)

# in CI the branch tip is the whole change; locally the working tree also holds
# staged and unstaged edits, and a pre-commit hook has to see those too — a
# two-dot diff against the merge base covers all three
if [[ -n "${CI:-}" ]]; then
    range=("$merge_base" "HEAD")
else
    range=("$merge_base")
fi

# M(odified), D(eleted) and R(enamed) all change or remove a checksummed file;
# A(dded) is the only legitimate way to touch this directory
changed=$(git diff --name-status --diff-filter=MDR "${range[@]}" -- "$dir")

if [[ -n "$changed" ]]; then
    cat >&2 <<EOF
error: shipped migrations were modified, deleted or renamed:
$changed

sqlx checksums every applied migration, so any existing deployment fails to
start on its next upgrade. revert the change and add a new NNNN_*.sql instead.
EOF
    exit 1
fi

echo "migrations are append-only: ok"
