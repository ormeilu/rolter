# Disaggregated prefill/decode routing belongs to the engine, not the gateway

**Status:** Accepted · **Date:** 11 Aug 2026 · **Issues:** [#853](https://github.com/rolter-ai/rolter/issues/853)
**Relates:** ADR-0014 (protocol translation boundary), ADR-0007 (cache-aware balancing)

## Context

#853 carried three unclaimed ideas from [llm-d](https://github.com/llm-d/llm-d).
The second — routing by inference phase to specialised pools — was flagged as
"the largest of the three and the one most coupled to how the fleet is
deployed", with the note that it may not fit rolter's position in the stack and
that the useful output in that case is a written "no, because". This is that
note.

Disaggregated serving splits inference in two. **Prefill** processes the prompt,
is compute-bound, and produces a KV cache. **Decode** generates tokens, is
memory-bandwidth-bound, and consumes that KV cache. Running them on separately
sized pools improves utilisation, because the two phases stress different
resources and otherwise contend on the same accelerator.

Making that work requires, per request: selecting a prefill worker, running
prefill there, **transferring the resulting KV cache to a decode worker**, and
then streaming generated tokens from the decode worker.

## What rolter is

ADR-0014 fixed the boundary: `rolter-gateway` owns authentication, tenancy,
target selection, tracing, metrics and accounting; `rolter-proxy` owns the wire
protocol. rolter forwards one client HTTP request to one upstream and streams
the response back. Everything it routes on is visible in the OpenAI/Anthropic
HTTP surface or in signals an engine publishes over HTTP.

## Options considered

### Option 1 — Implement P/D disaggregation in rolter

The gateway selects a prefill target and a decode target, drives both, and
coordinates the KV handoff between them.

### Option 2 — Treat a disaggregated fleet as one upstream

The engine-side coordinator owns phase routing and the KV transfer. rolter
balances across the fleet's entry points exactly as it does for any other pool,
and does not know a request was split.

### Option 3 — Hybrid: rolter routes phases, engine transfers KV

rolter picks both workers and issues two calls, but the KV cache moves over the
engine's own sidechannel rather than through rolter.

## Comparison

| Option | Pros | Cons |
|--------|------|------|
| **1. Implement in rolter** | Full control over phase placement; phase-aware scoring composes with existing scorers | Requires speaking an engine-specific KV transport (NIXL/UCX), none of which is part of any HTTP API; the data plane would hold per-request state across a handoff; couples rolter to one engine's internals and version cadence; duplicates work vLLM and llm-d already do |
| **2. One upstream** | Preserves the ADR-0014 boundary; works today with no code; every engine's own disaggregation implementation is usable, including future ones | rolter cannot influence phase placement, so it cannot improve on the engine's own decisions |
| **3. Hybrid** | Avoids moving KV through rolter | Still needs rolter to know which workers can hand off to which, to speak the connector's handshake, and to keep the request alive across two upstream calls — most of option 1's coupling for part of its benefit; a mid-request failure has no clean recovery |

## Decision

**Option 2.** rolter will not implement disaggregated prefill/decode routing. A
P/D-disaggregated deployment is a single upstream fleet from rolter's point of
view, and its own coordinator owns phase selection and the KV handoff.

## Rationale

The KV handoff is the whole feature, and it is not expressible over HTTP. It
moves gigabytes of tensor data between accelerators over RDMA-class transports
(NIXL, UCX) chosen by the engine. For rolter to place phases it would have to
speak that transport, or at minimum its connector handshake — putting an
engine-specific, version-coupled binary protocol inside a data plane whose
entire design premise is that it only speaks the two dominant HTTP dialects.
That is precisely the coupling ADR-0014 was written to prevent.

It would also be redundant. vLLM ships disaggregated serving with a connector
API, and llm-d's router is co-designed with its engine and scheduler — it can
assume things about worker topology and KV placement that a general-purpose
gateway in front of heterogeneous providers cannot. rolter's value is that it
sits in front of *many* engines and hosted APIs; a feature that only works for
one engine, at one version, with one transport configured, is not a good trade
against that.

The distinguishing question is: **does the signal reach rolter over HTTP?** For
prefix-cache affinity, queue depth, KV-event residency and LMCache occupancy the
answer is yes, which is why ADR-0007 and ADR-0021 exist and why LoRA-adapter
affinity (#853 part 1) was straightforward. For a KV cache transfer between two
accelerators the answer is no, and the boundary holds.

## Consequences

**Benefits:**

- the ADR-0014 boundary is preserved: no engine-specific binary transport in the
  data plane, and no per-request state held across an upstream handoff;
- disaggregated fleets are usable with rolter today, with no code, for any
  engine that implements disaggregation — including engines that do not exist
  yet;
- rolter does not inherit the failure modes of a mid-request handoff, which has
  no clean recovery once prefill has completed and decode is unreachable.

**Drawbacks and risks:**

- rolter cannot improve on the engine's phase placement, and cannot report on
  it beyond whatever the fleet exposes over its metrics endpoint;
- if an engine ever exposes phase-aware placement *over HTTP* — for example a
  documented header or a prefill-affinity hint on the OpenAI surface — this
  decision should be revisited, because the reason for it would no longer hold.

**System impact:**

None. This is a decision not to build something; no code changes.

## Related records

- ADR-0007 — Approximate cache-aware balancing behind a pluggable trait
- ADR-0014 — Extensible API protocol translation
- ADR-0021 — External cache telemetry for routing

## Open questions

- Should rolter document a recommended topology for putting a disaggregated
  fleet behind it (which entry point to target, and how to configure health
  checks against a coordinator rather than a worker)? That is documentation
  rather than a routing feature, and would be useful independently of this
  decision.
