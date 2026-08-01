# Load balancing

Each route maps a public model name to one or more upstream targets and a strategy. Strategies implement `rolter_balancer::LoadBalancer`:

```rust
pub trait LoadBalancer: Send + Sync {
    fn name(&self) -> &'static str;
    fn pick(&self, ctx: &RouteContext, loads: &[u64]) -> Option<usize>;
    fn observe(&self, target: usize, ctx: &RouteContext) {}
}
```

`pick` returns an index into the route's targets; `observe` lets learning strategies (cache-aware) record what a target served. `RouteContext` carries an optional `session_key` (from `x-session-id`) and the request `prompt` used for affinity scoring.

## Strategies (v1)

- **round_robin** — sequential rotation; predictable, zero state.
- **random** — uniform random; good for simple homogeneous pools.
- **power_of_two** — pick the less loaded of two random targets; needs a load snapshot.
- **consistent_hash** — hash-ring keyed by `session_key` (falls back to prompt hash); pins a session/user to a target for KV reuse, survives target changes with minimal reshuffle (160 vnodes).
- **cache_aware** — approximate prefix affinity; see [caching.md](caching.md).
- **weighted** — smooth weighted round-robin honouring each target's `weight`.
- **pipeline** — composable **filter → weighted-score → argmax** selection: eligibility filtering drops ineligible targets, then a stack of `Scorer`s (session affinity + static weight + in-flight load + prefix-cache affinity) is combined as a weighted sum and the argmax wins (ties broken randomly). Session affinity pins repeat requests from the same `x-session-id` to their last-served target (TTL-bounded) for warm-cache reuse. The extension point every future cost/latency/KV-cache scorer plugs into.
- **precise_cache_aware** — consumes each target's vLLM ZMQ KV-event stream and scores the exact leading fraction of caller-supplied token blocks resident on that target. Missing token ids and stale, malformed, disconnected, or sequence-gapped streams stay neutral; least-load routing remains the fallback.
- **lmcache_aware** — polls each target's configured LMCache controller signal and prefers available caches with free capacity (`1 - occupancy`). Empty, saturated, failed, and stale controllers stay neutral and fall back to least load.
- **adaptive** — a weighted blend of observed latency, catalog cost and in-flight load, governed by the deployment-wide `[adaptive_routing]` policy. See below.

## Adaptive routing

`adaptive` is the only strategy whose behavior is owned by a global policy rather than the route:

```toml
[adaptive_routing]
enabled = false           # kill switch; the whole feature is off by default
latency_weight = 1.0
cost_weight = 0.5
load_weight = 0.25
exploration_ratio = 0.05  # clamped to [0, 0.5]
min_samples = 50
```

Three conditions must all hold before the blend routes a single request: the kill switch is on, at least one weight is non-zero, and the route has both served `min_samples` requests and gathered latency samples for at least two targets. Until then — and immediately again if the policy is switched off — every pick goes to the same `pipeline` stack the route would have used otherwise, so moving a route to `adaptive` shifts no traffic on its own. The fallback stack keeps learning while the blend is engaged, so disengaging lands on a warm session/prefix cache.

The policy is also owned by the control plane: `GET`/`PUT /api/v1/adaptive-routing-policy` (superadmin only) persists it, audits the change as `adaptive_routing_policy.update`, and returns the `affected_routes` the change reaches. It travels to the data plane in the normal snapshot, so a change applies without a restart. The API refuses an all-zero blend outright — stopping adaptive routing is what `enabled = false` is for.

Once engaged, an `exploration_ratio` share of picks is made uniformly at random so a target the blend has learned to avoid keeps producing fresh latency samples instead of going dark. Operator input is clamped on the way in: negative weights become zero and exploration never exceeds half the traffic.

### Telemetry

The scores the blend ranks on live in the balancer, which lives in the gateway process, so the control plane cannot read them directly. Each gateway therefore **pushes** a sample every 15s to `POST /internal/adaptive-telemetry` on the control plane — the same internal token and the same `x-rolter-node-id` identity as the snapshot poll, so one node is one row here and in the cluster inventory (#543). Nothing is written on the request path: the sample is taken by a background task, and a pick costs two relaxed atomics for per-target attribution.

Per route and target the sample carries the blended score, its latency/cost/load components, the raw signals behind them (smoothed latency in ms, catalog price, in-flight count), how many picks that target has served and how long ago the last one was — plus the decision split, and the *sanitized policy that node actually runs*, which can lag the stored policy until the node converges.

The control plane keeps the newest sample per `(node, model)` in `adaptive_routing_telemetry` and serves `GET /api/v1/adaptive-routing-telemetry` (superadmin only), grouped by route with one entry per reporting node. Samples older than 60s are excluded as no longer current, and rows are pruned after an hour so a scaled-down node leaves the scoreboard.

This is deliberately **current state, not history**: one row per node and route, overwritten on every report, so the table is the size of the fleet rather than of the traffic. A time series belongs in the request log, and would be a separate endpoint rather than a change to this one. Prometheus deployments get the same per-target scores without the dashboard from `rolter_adaptive_routing_target_score{model,target}`.

## Choosing a strategy

| Use case | Strategy |
| --- | --- |
| Homogeneous pool, stateless | `round_robin` / `random` |
| Variable request durations | `power_of_two` |
| Multi-turn chat, sticky session | `consistent_hash` |
| Shared system prompts / few-shot / RAG | `cache_aware` |
| Blend cache + load + weight signals | `pipeline` |
| vLLM fleet with KV event publishing | `precise_cache_aware` |
| LMCache fleet with occupancy controller | `lmcache_aware` |
| Mixed-price providers, minimize spend | `cheapest` |
| Heterogeneous pool, minimize latency | `fastest` |
| Mixed price *and* latency, let the gateway tune | `adaptive` |

Both external strategies perform network I/O only in background tasks. The request hot path reads bounded in-process state and atomics.
