# Packaging & distribution

rolter ships three ways.

The unified `rolter` binary dispatches to both planes via subcommands:

```bash
rolter gateway --config rolter.toml     # data plane
rolter control --database-url postgres://…   # control plane + UI host
```

The standalone `rolter-gateway` / `rolter-control` binaries remain available.

## cargo

```bash
cargo install rolter            # unified launcher (from crates.io)
# or from source:
cargo install --path crates/rolter
```

## uv (PyPI wheel via maturin)

The wheel bundles the compiled `rolter` launcher so Python users can install the CLI with `uv`. `pyproject.toml` uses the maturin backend (`bindings = "bin"`, `manifest-path = crates/rolter/Cargo.toml`).

```bash
uv tool install maturin       # one-time
uvx maturin build --release   # build a wheel into target/wheels/
uv tool install rolter        # once published to PyPI
```

## Docker

Multi-stage `docker/Dockerfile` builds the Rust binaries and the Bun-built UI, then assembles a slim runtime:

```bash
docker build -f docker/Dockerfile -t rolter:dev .
docker compose -f docker/docker-compose.yml up -d          # full stack with postgres/redis/clickhouse
```

## Release pipeline

Releases are fully automated from Conventional Commits. Two workflows do the
work, and the handoff between them is the part worth understanding.

```text
merge to master
      │
      ▼
release-plz.yml ── release-pr ──►  "Release PR" (version bump + changelogs)
      │
      │ (that PR is merged)
      ▼
release-plz.yml ── verify ──► release ──►  crates.io publish
                                           tag v{version}
                                           github release
      │
      │ workflow_dispatch -f tag=v{version}      ← dispatch-artifact-release
      ▼
release.yml ── verify ─┬─► build-wheels (linux x86_64/aarch64, macos, windows)
                       ├─► publish-pypi   (trusted publishing, OIDC)
                       ├─► publish-docker (GHCR + optional Docker Hub)
                       └─► verify-parity  (all channels serve {version})
```

### Why the explicit dispatch

`release.yml` also has a `push: tags` trigger, but it never fires for a real
release. release-plz creates the tag with the repository `GITHUB_TOKEN`, and
[GitHub suppresses downstream workflow events for token-created refs][gh-token]
to prevent recursive runs. `workflow_dispatch` is the documented exception — it
always creates a run, even from the `GITHUB_TOKEN` — so `release-plz.yml` ends
with a `dispatch-artifact-release` job that calls
`gh workflow run release.yml -f tag=vX.Y.Z`. That job holds `actions: write` and
nothing else, and it fails if the dispatch produces no run.

Without it the pipeline half-works in the worst way: the GitHub release and
crates.io advance while no wheel is ever built, and every job stays green. That
is how v0.0.6 through v0.0.10 shipped while PyPI sat on 0.0.5 ([#903]).
`scripts/check-release-handoff.sh` (a merge gate in `quality.yml` and a prek
hook) asserts the wiring is still in place.

[gh-token]: https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow#triggering-a-workflow-from-a-workflow
[#903]: https://github.com/rolter-ai/rolter/issues/903

### Parity gate

`verify-parity` runs with `always()` at the end of `release.yml` and asserts
that every enabled channel actually serves the tagged version: the GitHub
release exists, crates.io has `rolter {version}`, PyPI has it (when
`PYPI_PUBLISH_ENABLED` is `true`), and GHCR has the manifest (when
`DOCKER_PUBLISH_ENABLED` is `true`). A skipped or failed publish turns the run
red instead of quietly leaving a channel behind.

### Publishing gates

| Gate | Effect |
|---|---|
| `verify` (reusable `quality.yml`) | the tagged commit passes the same fmt/clippy/test/deny pipeline as CI |
| `verify-external-checks` | CodeQL reported success for the commit; fail-closed |
| `RELEASE_REQUIRED_CHECKS` repo variable | exact check-run names the gate above requires (comma-separated) |
| `PYPI_PUBLISH_ENABLED` repo variable | must be `"true"` or the PyPI publish is skipped |
| `DOCKER_PUBLISH_ENABLED` repo variable | must be `"true"` or the image publish is skipped |
| `pypi` environment | PyPI trusted publishing via OIDC; no long-lived token is stored |

Wheels are built with `maturin-action` but uploaded with `pypa/gh-action-pypi-publish`:
`maturin upload` is deprecated and slated for removal ([PyO3/maturin#2334]). The
publisher identity PyPI matches on is the repository, workflow filename and
environment — not the tool — so the swap is transparent to the trusted-publisher
config, and it adds PEP 740 attestations on the `id-token` grant the job already
holds.

[PyO3/maturin#2334]: https://github.com/PyO3/maturin/issues/2334

`RELEASE_REQUIRED_CHECKS` holds exact check-run *names*, so it rots whenever a
scanner is renamed or reconfigured — and since the gate is fail-closed, a stale
name silently blocks every release instead of failing at the source. This bit
rolter once already: the variable still named the CodeQL *default setup* jobs
(`Analyze (rust)`, …) after the repo moved to advanced setup (`codeql (rust)`,
…), so no release could publish even with a working tag dispatch. If the gate
reports "required check … not found", compare it against the check-run names
the job log prints and update the variable.

### Releasing a tag by hand

For a backfill, or if a dispatch was lost, run the artifact half yourself from
the default branch:

```bash
gh workflow run release.yml -f tag=v0.0.10
gh run watch "$(gh run list --workflow=release.yml --limit 1 --json databaseId -q '.[0].databaseId')"
```

The upload uses `--skip-existing`, so re-running a tag that partly published is
safe. Verify the result:

```bash
curl -s https://pypi.org/pypi/rolter/json | jq -r .info.version
uv tool install rolter && rolter --version
```

### Backfill policy

Only the **current** release is backfilled to PyPI. The versions the broken
handoff skipped (v0.0.6 – v0.0.9) stay unpublished: they are superseded pre-1.0
releases, `uv tool install rolter` / `pip install rolter` resolve to the latest
version regardless, and publishing them retroactively would put four versions on
the index that no user ever pinned, dated years after their tags. Anyone who
needs one of them can build from the tag or `cargo install rolter@0.0.x` from
crates.io, which has the complete series.
