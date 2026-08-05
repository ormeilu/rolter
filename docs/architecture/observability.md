# Observability

## Metrics

- The gateway exposes Prometheus metrics at `GET /metrics`: counters (`rolter_requests_total`, `rolter_upstream_errors_total`, `rolter_auth_failures_total`, reload/log/budget/rate-limit/retry/cooldown/health/breaker/scrape counters), a `rolter_config_version` gauge, and per-model **latency histograms** — `rolter_request_latency_ms` (total) and `rolter_request_ttft_ms` (time-to-first-token), each labelled `{model=...}` with the standard `_bucket`/`_sum`/`_count` series. Histograms are observed once per completed request from the log sink, off the response hot path.
- The exporter is hand-rolled (atomic counters + non-cumulative histogram buckets cumulated at render) rather than the `metrics` facade + global recorder, which does not fit the lock-free `arc-swap` design where an explicit `Arc<Metrics>` is threaded through the request path.
- Passive per-target SLA signal: `rolter_target_requests_total{provider,target,outcome}` (a counter, `outcome` = `ok` for 2xx else `error`) is tallied once per completed request from the log sink — free, derived from real traffic, no extra upstream calls. A per-target error rate / uptime is `sum(rate(rolter_target_requests_total{outcome="error"}[5m])) / sum(rate(rolter_target_requests_total[5m]))`. This is the first slice of provider stability tracking (ROL-123); the ClickHouse `provider_health_events` table and the dashboard land in later slices. The active prober is guarded: bounded probe concurrency with per-provider jitter, consecutive-failure/-recovery thresholds gating the unhealthy flip (no single-probe flapping), and exponential probe backoff when a probe itself gets a 429.
- Multi-key providers: `rolter_key_cooldowns_tripped_total` counts api keys parked after a key-level failure (429/401 on a provider with several keys); the request retries in-flight on a sibling key.
- A/B attribution: `rolter_variant_requests_total{model,variant}` (a counter) tallies requests per chosen variant, so traffic splits are visible in Prometheus/Grafana without querying ClickHouse. Classic single-pool routes (no variant) emit nothing. Observed from the same log-sink funnel (ROL-195, part of ROL-188).
- Adaptive routing (#544): `rolter_adaptive_routing_decisions_total{model,mode}` splits a route's picks between `blend` (the latency/cost/load blend), `exploration` (the bounded random share that keeps starved targets sampled) and `fallback` (the deterministic `pipeline` stack served while the kill switch is off or the evidence is too thin), and `rolter_adaptive_routing_engaged{model}` is `1` while the blend is actually routing. Only routes on the `adaptive` strategy emit these. A config reload rebuilds the balancer and so resets the counters, which lines up with a `rolter_config_version` bump — alert on `rate()`, not the absolute value. A route sitting at `engaged 0` with all picks on `fallback` is the expected steady state before an operator enables the policy. `rolter_adaptive_routing_target_score{model,target}` (#751) adds the blended score each target currently carries — the same numbers the control plane serves at `GET /api/v1/adaptive-routing-telemetry`, computed at scrape time rather than on the request path.
- Roadmap: add per-provider/route labels on the histograms, in-flight gauges, cache-hit ratio, and circuit-breaker state gauges.
- Roadmap: **scrape/federate upstream engine metrics** from vLLM/SGLang/TGI `/metrics` and correlate them per target (queue depth, KV-cache usage, running/waiting requests) to feed load- and cache-aware routing and the dashboard.

## Tracing & context propagation

- `tracing` + `tracing-subscriber` with `RUST_LOG` filtering; `TraceLayer` logs each HTTP request.
- **Inbound**: accept W3C `traceparent`/`tracestate` (and `b3`) from clients and continue the trace; honor `x-request-id` / `x-correlation-id`.
- **Outbound to engines**: inject the active trace context into upstream requests so vLLM/SGLang/TGI spans join the **same** distributed trace. vLLM and SGLang support OpenTelemetry tracing (e.g. vLLM `--otlp-traces-endpoint`); point them at the same OTLP collector so engine prefill/decode spans line up with rolter's request span.
- A per-request `request_id` is echoed in a response header and stamped on logs, metric exemplars and spans for correlation.

### How the context actually moves

`rolter-core::telemetry` owns both directions, and both are inert unless an OTLP
endpoint is configured:

- **Extract.** `GatewayMakeSpan` (`rolter-gateway::trace`) is the `TraceLayer`'s
  span-maker: it builds the request span and makes the extracted inbound context
  its *parent*. A B3-only caller is normalized into an equivalent `traceparent`
  first, so one W3C propagator serves both wire formats. Without this the
  gateway's spans were disconnected roots — the trace id reached the request log,
  but nothing joined the caller's trace.

  It has to be the span-maker rather than a middleware layered inside the
  `TraceLayer`: `DefaultMakeSpan` builds the request span at **DEBUG**, so under
  the default `RUST_LOG=info` it is disabled, and setting a parent on a disabled
  span silently does nothing. With no pipeline installed it falls back to that
  stock DEBUG span, so the untraced path costs what it always did.
- **Inject.** The context handed to the provider is injected from the *current*
  span, inside the per-attempt `upstream.request` span, rather than copied from
  the caller. Copying it verbatim made the provider call a child of the caller's
  span and therefore a **sibling** of the gateway's own work, which silently
  invalidated every waterfall built from the data. The allowlisted client headers
  from `Forwarder::forwarded_header_names` are unaffected; only the trace headers
  changed hands.
- **Log correlation.** `RequestLog.trace_id` is read off the span context when a
  pipeline is installed and falls back to parsing the inbound header otherwise,
  so ClickHouse and the trace backend agree by construction.

### Pipeline spans

One span per stage, so a slow request is attributable rather than merely slow:

| Span | Attributes |
|---|---|
| `auth` | — |
| `guardrails.pre` | `redacted`, `webhook` |
| `route.select` | `route`, `strategy`, `candidates` |
| `cache.lookup` | `hit`, `kind` (`exact` / `semantic`) |
| `queue.wait` | `provider` |
| `upstream.request` | `attempt`, `gen_ai.system`, `gen_ai.request.model`, `http.response.status_code` |
| `translate.request` | — |
| `guardrails.post` | — |

`queue.wait` spans enqueue→dequeue only: the span travels with the queued job and
the worker closes it the moment it picks the job up, so it measures the wait and
not the wait plus the upstream call. The job carries the caller's span alongside
it, and the worker instruments the forward with it — the queue worker runs on its
own task where nothing is in scope, so without that the forwarder's own
`translate.request` span becomes an orphan root in a trace of its own. Names
follow the OTel
[GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/)
where they fit, so a backend's built-in GenAI views work.

Spans never carry prompt or completion content, API keys, virtual-key plaintext,
or injected header values — those are credential material, and redaction stays
owned by the existing `logging_settings` machinery rather than a second policy.

### Running it locally

The observability overlay starts a collector and a trace UI alongside the normal
stack:

```
docker compose -f docker/docker-compose.yml \
               -f docker/docker-compose.observability.yml up
```

Traces land at <http://localhost:16686>; the collector takes OTLP on 4317/4318
and re-exposes collected metrics on 8889. The overlay sets
`OTEL_EXPORTER_OTLP_ENDPOINT` on the `gateway` and `control` services, so
bringing it up is the only step — without it that variable is unset and tracing
stays off.

The collector binds `0.0.0.0`, not `localhost`: one bound to loopback inside its
container is unreachable from the gateway container.

#### Choosing an overlay

There are two, and they are mutually exclusive — both publish OTLP on 4317/4318.

| | `docker-compose.observability.yml` (default) | `docker-compose.signoz.yml` |
|---|---|---|
| backend | Jaeger v2 | SigNoz |
| signals | traces only | traces, metrics, logs |
| containers | 2 | 5, incl. its own ClickHouse + Zookeeper |
| storage | in memory, lost on restart | persistent |
| use it for | reading a waterfall, fast iteration | aggregate views, dashboards, dogfooding over time |

```
docker compose -f docker/docker-compose.yml \
               -f docker/docker-compose.signoz.yml up      # SigNoz on :8080
```

Jaeger is the default because it is two containers and starts in seconds, and
because reading a correctly-parented waterfall is what the tracing work needed.
Reach for SigNoz when "which stage is slow *across all requests*" matters, which
Jaeger cannot answer.

Nothing in the Rust differs between them: the gateway speaks vendor-neutral OTLP
and only the destination changes. Any other OTLP backend works the same way —
repoint the exporter in `infra/otel/collector.compose.yaml`.

#### Querying SigNoz from an agent (MCP)

The SigNoz overlay also starts SigNoz's MCP server on `http://localhost:8000/mcp`,
so an agent can query traces, run ClickHouse queries, and manage dashboards
against the local instance. Point an MCP client at that URL; the
[SigNoz agent-skills plugin](https://github.com/SigNoz/agent-skills) ships a
`signoz` server entry to fill in.

It needs a SigNoz API key, which is created in the UI (**Settings → API Keys**,
admin only) and supplied to the *server*, not the client:

```
set -x SIGNOZ_API_KEY (pass show rolter/signoz-api-key)
docker compose -f docker/docker-compose.yml \
               -f docker/docker-compose.signoz.yml up -d signoz-mcp
```

The key is read from the environment and never written to a tracked file. With
no key set the container still starts, but every call returns
`Authorization or SIGNOZ-API-KEY header required`.

The MCP dashboard tools need SigNoz v0.135.0 or newer, which is why the `signoz`
image is pinned ahead of the collector/ClickHouse pair.

The SigNoz overlay is a pinned equivalent of what SigNoz's Foundry CLI generates,
since SigNoz deprecated its own compose manifests in v0.130.0. Two things to know
before touching it: its four images are a **tested set** and must be bumped
together (a newer migrator emits ClickHouse settings an older server rejects),
and it vendors 3.6 KB of ClickHouse config — the cluster topology and the
`{shard}`/`{replica}` macros that `ReplicatedMergeTree` needs — rather than
SigNoz's full 56 KB `config.xml`, which the stock image defaults cover.

### Cost when tracing is off

With no `OTEL_EXPORTER_OTLP_ENDPOINT` (the default) behaviour and hot-path cost
are unchanged: `telemetry::is_active()` is a single relaxed atomic load, stage
spans are `Span::none()` (no allocation, and instrumenting a future with one is a
no-op), no carrier is built, and outbound trace headers are copied verbatim
exactly as before.

## Exporters (OTel-compatible)

rolter emits traces and metrics via **OpenTelemetry OTLP** (gRPC/HTTP), so any OTel-compatible backend works without code changes — just set an endpoint and headers:

- **SigNoz**, **Grafana Tempo/Mimir**, **Honeycomb**, **Datadog** (OTLP intake or the OTel Collector `datadog` exporter).
- **Langfuse** for LLM-specific observability (prompt/response, token usage and cost as traces), ingested via its OTLP endpoint or SDK.

Recommended topology: rolter → **OpenTelemetry Collector** → fan-out to the chosen backends. The collector also scrapes the upstream engines' `/metrics` and rolter's `/metrics`, keeping vendor specifics out of rolter. Configure via env, e.g. `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME=rolter-gateway`.

[`infra/otel/collector.yaml`](../../infra/otel/collector.yaml) is a ready-to-run
local collector example: it accepts OTLP/gRPC and OTLP/HTTP, scrapes the
compose gateway's Prometheus endpoint, exposes collected metrics on `:8889`,
and prints telemetry via the `debug` exporter. It intentionally has no external
backend configured, keeping the default example safe for air-gapped use. Replace
the `debug` exporter with an internal OTLP-compatible destination for production.

## Request & cost logs

- Every proxied request is logged to **ClickHouse** (`request_logs`): identifiers, model, provider/target, status, token counts, `cost_usd`, latency, TTFT, cache flag, error.
- **Retention** defaults to 90 days for metadata and seven days for captured payloads, set as the TTL in the ClickHouse schema. Both are admin-managed: `PUT /api/v1/logging-settings` accepts `retention_days` (1–3650) and `payload_retention_hours` (1–8760) and issues the matching `alter table … modify ttl` against ClickHouse, which then expires parts on its own schedule. Payload retention may not exceed metadata retention, so raw prompt bodies never outlive the row they belong to. A ClickHouse failure leaves the stored policy in place and is logged rather than failing the admin write — re-saving reapplies it.
- **Payload capture** is disabled by default. Set `[logging.payload_capture] enabled = true` to write redacted request and response payloads to the separate `request_payloads` table, which has a seven-day TTL (versus 90 days for request metadata). `max_bytes` bounds each body; `redact_fields` adds recursively redacted JSON keys before storage. Optional `models` and `virtual_key_ids` allow-lists make the deployment-level switch route- or key-specific.
- **Request id / trace continuation**: every request carries an `x-request-id` — the caller's when supplied, otherwise a generated UUID — which is echoed on the response and stored on the log row for end-to-end correlation. An inbound W3C `traceparent` or B3 (`b3` / `x-b3-traceid`) header is parsed and its trace id stored in `request_logs.trace_id`, so gateway logs join the caller's distributed trace instead of starting a disconnected one.
- **Outbound propagation**: when the caller sent trace context, it is forwarded verbatim to the chosen upstream (`traceparent`, `tracestate`, and the `b3` / `x-b3-*` family) so vLLM/SGLang/TGI continue the same trace. An untraced request adds nothing to the upstream wire — this is the caller's own context, not a rolter fingerprint, so it preserves wire transparency.
- Writes are **async and batched off the hot path** so logging never adds request latency.
- The dashboard queries ClickHouse for usage, spend, latency percentiles and error rates, sliced by org/team/project/key/model.

## Provider health events

- Every health signal is written to **ClickHouse** (`provider_health_events`): `target_id`, `provider`, `source`, `outcome`, `status_code`, `latency_ms`, `error_kind`, timestamped by ClickHouse on insert.
- `source` distinguishes where the observation came from: `passive` (real traffic completing through the request funnel), `probe` (active liveness sweeps), and the opt-in `llm_call` / `status_page` sources.
- `outcome` is `ok` / `error` / `timeout`; `error_kind` gives a coarse label (`rate_limited`, `upstream_error`, `connect_error`, `timeout`).
- Writes reuse the same **async, batched, off-hot-path** writer and ClickHouse endpoint as `request_logs`; when no `clickhouse_url` is configured the sink is a no-op.
- Counters `rolter_health_events_written_total` and `rolter_health_events_dropped_total` track the writer, mirroring the request-log counters.
- This event stream feeds uptime %/MTTR rollups and the dashboard health panel.

### Stability rollup API

Read-only, window-bounded rollups over `provider_health_events`, served by the control plane when `--clickhouse-url` is set (otherwise `503`). All accept `since`/`until` (RFC3339, default last 7 days); time bounds are passed as ClickHouse query parameters, never interpolated.

- `GET /api/v1/health/uptime` — per provider/target: event counts, `uptime`, `failure_rate`, `error_budget_burn` and `sla_breached` against an `sla` target (query param, fraction in `(0,1]`, default `0.99`), and `last_event`.
- `GET /api/v1/health/mttr` — per provider/target mean time to recovery (`mttr_seconds`) and incident count, computed from downtime episodes (a run of non-`ok` events bounded by `ok`).
- `GET /api/v1/health/timeline?bucket=hour|day|week|month` — bucketed ok/error/timeout counts per provider/target for the failure timeline (default bucket `hour`).

## Health

- `GET /healthz` on both binaries for liveness probes.
- `GET /readyz` on the gateway for readiness. It returns `503 draining` once the control plane marks the node as draining (`PUT /api/v1/cluster/nodes/{id}/drain`), so a load balancer stops sending new traffic while in-flight requests finish; `/healthz` stays `200` because the process is healthy. The drain reaches the node on the snapshot poll it already makes, and the control plane refuses to drain the last live gateway.
