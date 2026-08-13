#!/usr/bin/env bash
# the release pipeline hands off in two hops, and both are easy to delete by
# accident because neither has a test behind it:
#
#   release-plz.yml  --workflow_dispatch-->  release.yml  --> wheels/pypi/ghcr
#
# release-plz tags with the repo GITHUB_TOKEN, and GitHub suppresses downstream
# events for token-created refs, so release.yml's `push: tags` trigger never
# fires for a real release. `workflow_dispatch` is the documented exception —
# it always creates a run. drop that dispatch and releases keep going out with
# a github release and crates.io but no wheel, silently and forever: that is
# exactly what happened to v0.0.6-v0.0.10 while pypi sat on 0.0.5 (#903).
#
# this check asserts the handoff and its parity gate are still wired up.
set -euo pipefail

plz=".github/workflows/release-plz.yml"
rel=".github/workflows/release.yml"
fail=0

require() {
    # $1 file, $2 human description, $3 grep pattern
    if ! grep -Eq "$3" "$1"; then
        echo "error: $1: $2" >&2
        fail=1
    fi
}

for f in "$plz" "$rel"; do
    if [[ ! -f "$f" ]]; then
        echo "error: $f is missing; the release pipeline needs both halves" >&2
        exit 1
    fi
done

# ── the handoff itself ──────────────────────────────────────────────────────
require "$plz" "no 'dispatch-artifact-release' job — release.yml will never run" \
    '^  dispatch-artifact-release:'
require "$plz" "the handoff job no longer dispatches release.yml" \
    'gh workflow run release\.yml'
require "$plz" "the handoff job needs 'actions: write' to dispatch" \
    '^      actions: write'
require "$plz" "the handoff must run after release-plz-release, on its tag output" \
    'needs: release-plz-release'
require "$plz" "release-plz-release must expose the resolved tag as a job output" \
    '^      tag: \$\{\{ steps\.tag\.outputs\.tag \}\}'

# ── the receiving end ───────────────────────────────────────────────────────
require "$rel" "release.yml dropped its workflow_dispatch trigger" \
    '^  workflow_dispatch:'
require "$rel" "release.yml dropped the 'tag' dispatch input" \
    '^      tag:'
require "$rel" "release.yml no longer has a publish-pypi job" \
    '^  publish-pypi:'
# a local reusable workflow checks out the caller's ref, which on a dispatch is
# master — not the tag being packaged. without this the gate verifies one tree
# and ships another (#988)
require "$rel" "the verify gate must be pinned to the packaged ref, not the caller's" \
    'ref: \$\{\{ inputs\.tag \|\| github\.ref \}\}'

# ── the parity gate ─────────────────────────────────────────────────────────
# without this a skipped publish drags the run to green instead of red
require "$rel" "release.yml dropped the verify-parity gate" \
    '^  verify-parity:'
require "$rel" "verify-parity must observe publish-pypi to catch a skipped publish" \
    'needs: \[verify, verify-external-checks, build-wheels, build-image, smoke-wheels, smoke-image, publish-pypi, publish-docker\]'
require "$rel" "verify-parity must run with always() or a skipped publish stays invisible" \
    'if: always\(\)'

# ── the artifact set ────────────────────────────────────────────────────────
# `macos-latest` is arm64, so without an explicit x86_64 target intel macs get
# no wheel; without an sdist they get no candidate at all and the install fails
# outright rather than building from source (#989)
require "$rel" "release.yml must cross-build the macos x86_64 wheel" \
    'target: x86_64-apple-darwin'
require "$rel" "release.yml must build an sdist as the source fallback" \
    '^          command: sdist'
require "$rel" "verify-parity must assert the published artifact set" \
    'expect "macos x86_64 wheel"'
require "$rel" "verify-parity must assert an sdist was published" \
    'expect "sdist"'

# ── the publish barrier ─────────────────────────────────────────────────────
# stage 1 builds everything, stage 2 publishes it. if a publish job stops
# depending on every build job, a failed wheel can leave images public against a
# release with nothing on pypi (#992)
require "$rel" "release.yml must build images per arch in a separate stage" \
    '^  build-image:'
require "$rel" "stage 1 must push untagged digests, not tags" \
    'push-by-digest=true'
require "$rel" "the publish jobs must wait for every build and smoke job" \
    '^    needs: \[verify, verify-external-checks, build-wheels, build-image, smoke-wheels, smoke-image\]'

# an artifact that was never run is not a verified artifact; these catch a
# release that publishes something it never installed or executed (#903)
require "$rel" "release.yml must install and run the built wheel before publishing" \
    '^  smoke-wheels:'
require "$rel" "release.yml must run the built image before publishing" \
    '^  smoke-image:'
require "$rel" "the wheel smoke test must resolve only from dist/, never pypi" \
    'no-index'

if [[ "$fail" -ne 0 ]]; then
    cat >&2 <<'EOF'

the release handoff is broken. see docs/development/packaging.md ("Release
pipeline"); a release that loses this wiring publishes a github release and
crates.io but never a pypi wheel, and nothing goes red.
EOF
    exit 1
fi

echo "release handoff is wired: ok"
