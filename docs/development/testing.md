# Testing

## Run

Tests run under [nextest](https://nexte.st/) (the same runner CI uses), plus a
separate doc-test pass since nextest does not run doc tests:

```bash
cargo nextest run --workspace   # unit + integration tests
cargo test --doc --workspace    # doc tests
cd ui && bun run lint           # ui typecheck
```

Install the runner once with `cargo install cargo-nextest` (or see the
[nextest install docs](https://nexte.st/docs/installation/)). `just test` runs
both Rust passes for you. Plain `cargo test --workspace` still works if you
haven't installed nextest, but CI runs nextest so prefer it locally.

The Ollama Cloud live smoke sends a billed request and is ignored by default:

```bash
OLLAMA_API_KEY=... ROLTER_OLLAMA_LIVE_MODEL=gpt-oss:20b \
  cargo test -p rolter-gateway --test ollama_cloud live_smoke -- --ignored
```

The Gemini Interactions smoke is gated the same way. It exists because Google
publishes no full JSON schema for the interactions wire format, so parts of the
adapter — the multimodal part field names and some `step.delta` variants —
were inferred from prose docs and only a real request confirms them (#764):

```bash
GEMINI_API_KEY=... ROLTER_GEMINI_LIVE_MODEL=gemini-3.6-flash \
  cargo test -p rolter-gateway --test gemini_interactions_live -- --ignored
```

Both run in CI only from dispatch-gated workflows, never the per-PR gate:
`quality.yml` takes no secrets by design (#734) so dependabot and fork PRs pass
exactly the same checks. Assertions in the live suites carry the upstream
response body in their failure message — with an inferred field name, the
provider's complaint *is* the finding, and a bare status-code assertion would
throw it away.

Test grouping is configured in [`.config/nextest.toml`](../../.config/nextest.toml):
the Postgres-backed `rolter-store`/`rolter-control` suites share one database and
reset the schema per test, so they run in a single-threaded group to avoid
clobbering each other.

## Layout

- **Unit tests** live next to the code in `#[cfg(test)] mod tests`. Current coverage: balancer strategies (round-robin cycling, consistent-hash stability, cache-aware affinity, empty targets), the prefix trie, config parsing, model rewrite, auth checks, and the in-memory store.
- Keep the pure crates (`rolter-core`, `rolter-balancer`, `rolter-auth`) fully unit-testable without I/O.

## Strategy as the project grows

- **Integration tests** for the gateway: spin up the Axum app with a mock upstream (`wiremock`/`httpmock`) and assert routing, auth, model rewrite, error mapping and streaming passthrough.
- **Property tests** (`proptest`) for the balancer: distribution fairness, affinity invariants.
- **DB tests** for `rolter-store` Postgres backend behind a feature, using a disposable container.
- **Load tests** (`oha`/`k6`) against a mock upstream to track added latency and max RPS (see [performance.md](../architecture/performance.md)).

## Chaos & resilience contracts

Resilience is asserted in two places, split by what each harness can drive
deterministically.

The **e2e chaos suite** (`integration/e2e/tests/test_chaos.py`, compose `chaos`
profile) drives a static-config gateway against mock upstreams whose failure mode
is fixed by an env var. It covers what is observable purely over the wire: retry
and failover on 5xx/429, a clean 5xx when every target is down, the request
timeout bound on a slow upstream, the circuit breaker's OPEN transition (asserted
via `rolter_breaker_opened_total`) and flap degrade/recovery.

The **gateway chaos tests** (`crates/rolter-gateway/tests/chaos.rs`) cover the two
contracts that need a request pinned at a known point inside the gateway, which a
black-box harness can only approximate with sleeps:

- **bounded-queue backpressure** — with `queue.capacity = 1`, `queue.workers = 1`
  and `backpressure = "error"`, one request pins the sole worker, exactly one
  surplus request takes the queue slot, and the rest must come back as
  `429` with `error.code = "queue_full"` while
  `rolter_provider_queue_rejections_total` advances. Memory is bounded by the
  queue, not by client burst size.
- **graceful SIGTERM drain** — a real `rolter-gateway` child process is sent
  `SIGTERM` while a request is pinned upstream. The in-flight request must still
  return `200`, new connections must be refused, and the process must exit `0`.

Both use a mock upstream that blocks on a semaphore the test owns, so every step
is driven by a signal rather than by elapsed time — there are no sleeps to race.
Run them with:

```bash
cargo test -p rolter-gateway --test chaos
```

## Benchmarks

Hot-path micro-benchmarks run under [criterion](https://github.com/criterion-rs/criterion.rs). They live in `crates/<crate>/benches/` with a `[[bench]] harness = false` entry per file, and cover the per-request cost that shows up as pure gateway overhead:

```bash
just bench                       # cargo bench --workspace
cargo bench -p rolter-balancer   # just the balancer benches
cargo bench -p rolter-balancer --bench pick   # one bench target
```

Current coverage:

`rolter-balancer`

- `pick` — `LoadBalancer::pick` for every built-in strategy over a ~24-target pool with a populated `RouteContext`.
- `trie` — prefix-trie `insert` (bounded/unbounded, so LRU eviction is measured) and `longest_prefix` on a warm trie.

`rolter-core`

- `snapshot` — the CPU side of config-snapshot generation at 10/100/1000 routes: `sanitize_for_snapshot`, `validate`, and the JSON encode. `/internal/snapshot` is polled by every gateway in the fleet, so this cost is paid fleet-wide on every poll. The encode dominates — ~2.8 ms at 1000 routes against ~150 µs for sanitize — which is why payload size is its own metric (#845).

criterion writes HTML reports to `target/criterion/`. Benches are **not** run in CI (timings are noisy on shared runners), but `cargo clippy --workspace --all-targets -- -D warnings` compiles them on every PR, so they cannot silently bit-rot. Use `just bench-check` (`cargo bench --workspace --no-run`) to compile them locally without running.

## Coverage

Workspace line coverage is measured with
[`cargo llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```bash
cargo install cargo-llvm-cov
cargo llvm-cov --workspace --all-features --summary-only   # quick %
cargo llvm-cov --workspace --all-features --html           # browsable report
```

CI runs coverage in the `coverage` job of `quality.yml` and enforces a
**ratcheting baseline**: the committed baseline lives in
[`.github/coverage-baseline.txt`](../../.github/coverage-baseline.txt), and
[`.github/scripts/coverage-ratchet.sh`](../../.github/scripts/coverage-ratchet.sh)
fails the step if the current percentage drops more than
`COVERAGE_TOLERANCE` points (default `0.5`) below it. The job also uploads the
`lcov.info` report as a CI artifact.

Policy (ROL-246):

- New code must not push coverage below `baseline − tolerance`. If a PR
  legitimately lowers coverage, edit `.github/coverage-baseline.txt` in the same
  PR and explain why.
- When coverage climbs well above the baseline, raise the baseline to lock in
  the gain (the ratchet only goes up).
- The job is **informational** (`continue-on-error: true`) until the baseline is
  trusted; promote it to blocking by removing that flag on the `coverage` job.

## CI

`.github/workflows/ci.yml` delegates to the shared `quality.yml` gate, which runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo nextest run --workspace --all-features` plus a `cargo test --doc` pass, the feature matrix, `cargo doc` (warnings as errors), cargo-deny, gitleaks, the UI lint/build, and a Conventional Commit PR-title check on every push/PR.

### Secret scanning

The `gitleaks` job runs the gitleaks **CLI** from a digest-pinned container, not
`gitleaks-action`. The action gates org-owned repositories behind a license key,
and license secrets are invisible to both dependabot runs (a separate secret
store) and fork PRs (no secrets at all), so every such PR failed the job and with
it `ci-ok`. The CLI is free and unrestricted, so `quality.yml` now takes no
secrets and behaves identically for forks, dependabot and direct pushes.

Two passes run with the shared `.github/config/gitleaks.toml` policy: `gitleaks
dir` over the working tree (everything the commit ships) and, on PRs, `gitleaks
git --log-opts base..head` over the branch history (catches a secret added and
then removed inside the same PR). The pinned digest is v8.30.1 — the version
`prek.toml` already uses for the staged-content hook, so local and CI scans agree.

Reproduce a CI run locally:

```bash
docker run --rm -v "$PWD:/repo" -w /repo \
  ghcr.io/gitleaks/gitleaks@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f \
  dir . --config .github/config/gitleaks.toml --redact --exit-code 1
```
### Storybook play tests

The `storybook` job builds the static Storybook, serves it, and runs the
interaction (play) tests with `@storybook/test-runner` against a headless
chromium. It is a **merge gate** (#753): a failing play test fails `quality`,
which fails `ci-ok`. Locally:

```bash
cd ui
bunx playwright install --with-deps chromium chromium-headless-shell
bun run build-storybook
python3 -m http.server 6006 --directory storybook-static &
bun run test-storybook --url http://127.0.0.1:6006
```

`ui/package.json` pins `playwright` and `playwright-core` through `overrides`.
The test-runner declares its own loose `playwright` range, so without the pin it
resolves a different version from `@playwright/test` and launches a browser
revision `playwright install` never downloaded — the test-runner then fails at
launch and the play tests silently stop running (#737). Keep both on one version,
and install `chromium-headless-shell` alongside `chromium`, since the test-runner
launches the shell rather than the full build.

The job is **informational** (`continue-on-error: true`) pending the ROL-124
promotion path.

### Full-stack compose smoke

The `compose-smoke` job boots the production-shaped Docker Compose topology
(Postgres, Redis, ClickHouse, gateway, control) and exercises it end-to-end. Run
it locally with the same script CI uses:

```bash
bash docker/smoke/smoke.sh
```

It layers [`docker/docker-compose.ci.yml`](../../docker/docker-compose.ci.yml)
over the base compose file: the overlay mounts
[`docker/smoke/rolter.smoke.toml`](../../docker/smoke/rolter.smoke.toml) (a
keyless open gateway config) so the built-in `fake-llm` model answers without any
provider secret. The script waits for both `/healthz` endpoints, checks
`/v1/models` and `fake-llm` chat (non-streaming + SSE) on the gateway and the
postgres-backed `/internal/snapshot` on the control plane, then always dumps
compose logs and runs `down -v`. It is **informational** (`continue-on-error`)
until the image-build cost and flake profile are trusted (ROL-245).
