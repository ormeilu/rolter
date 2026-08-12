# Dogfooding fleet

A local rolter with a fleet that looks like a real one, for operator dogfooding
(#924).

The built-in `fake-llm` model answers "does the gateway work at all". It does
not answer "what is it like to run this", because one route with one target
exercises none of the screens an operator lives in — provider groups, routing
strategies, per-target health, key scoping. Every strategy looks identical when
every target answers in the same time.

So this stands up fifteen fake upstreams with deliberately uneven latencies, one
route per strategy worth looking at, and the whole session traced into SigNoz.

## What is here

| File | What it is |
|---|---|
| `fleet.ts` | fifteen fake OpenAI-compatible upstreams on `127.0.0.1:18001-18015` |
| `dogfood.toml` | the matching rolter config — fifteen providers, eleven routes |
| `keys.env` | the API keys the fleet expects (fake, loopback-only, checked in on purpose) |

## The fleet

Three shapes, named the way each of them names things:

- **`:18001`** — OpenAI's model names (`gpt-4o`, `o3-mini`), added as an `openai` provider
- **`:18002`** — OpenRouter's `vendor/model` names, added as `openai_compatible` rather than `openrouter` (see #925)
- **`:18003-18015`** — a self-hosted vLLM/TEI fleet, one model per instance, roughly half behind a key

Three of them exist to make failure legible: `vllm-a100-03` is ~4x slower than
its pair, `vllm-spot-01` returns a 503 for a quarter of requests, and
`vllm-spot-02` takes ~1.4s to first token. A fleet with no bad targets leaves
the health, breaker and latency screens permanently green and unreadable.

## Running it

Bring up Postgres, Redis, ClickHouse and SigNoz:

```bash
docker compose -f docker/docker-compose.yml -f docker/docker-compose.signoz.yml \
  up -d postgres redis clickhouse signoz-zookeeper signoz-clickhouse \
        signoz-schema-migrator signoz-otel-collector signoz signoz-mcp
```

Start the fleet, then seed the database from the same config:

```bash
bun integration/dogfood/fleet.ts &
set -a; . integration/dogfood/keys.env; set +a
export ROLTER_DATABASE_URL=postgres://rolter:rolter@127.0.0.1:5432/rolter
cargo run -p rolter-control --features postgres --bin rolter-seed -- \
  --import integration/dogfood/dogfood.toml
```

> `--import` only inserts. It will not update a provider that already exists
> (#927), so drop the volume rather than re-importing over an edited file.

Run the control plane and the gateway, both exporting to the collector:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317
export ROLTER_REDIS_URL=redis://127.0.0.1:6379 CLICKHOUSE_URL=http://127.0.0.1:8123
OTEL_SERVICE_NAME=rolter-control ROLTER_UI_DIR=ui/dist \
  ROLTER_UI_OTEL_ENDPOINT=https://otel.localhost/v1/traces \
  cargo run -p rolter-control --features postgres --bin rolter-control &
OTEL_SERVICE_NAME=rolter-gateway \
  ROLTER_SNAPSHOT_URL=http://127.0.0.1:4001/internal/snapshot \
  cargo run -p rolter-gateway -- --config integration/dogfood/dogfood.toml &
```

`[logging].clickhouse_url` in `dogfood.toml` is what makes the dashboard's
analytics screens fill; the control plane's `CLICKHOUSE_URL` only lets it
*read* the table (#929).

## Browser tracing

The dashboard's own spans (#805) are posted from the operator's browser, so
the collector has to answer a cross-origin preflight and the page cannot be
`https` while the endpoint is `http`. Both are handled: the OTLP receiver in
`docker/signoz/otel-collector-config.yaml` allows loopback origins, and the
`.localhost` names below terminate TLS.

Without those, the spans are dropped by the browser before the collector sees
them and the only symptom is an empty `rolter-ui` service in SigNoz.

## URLs

With [portless](https://github.com/vercel-labs/portless):

```bash
portless alias rolter 4001 && portless alias api.rolter 4000
portless alias signoz 8080 && portless alias otel 4318
```

| URL | What |
|---|---|
| `https://rolter.localhost` | dashboard |
| `https://api.rolter.localhost` | gateway (`/v1/*`) |
| `https://signoz.localhost` | traces, metrics, logs |
