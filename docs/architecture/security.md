# Security

## Secret handling

- **Upstream provider keys** are never stored in plaintext in the database. They are **envelope-encrypted** with AES-256-GCM: a per-record data key/nonce, wrapped by a master key (KEK) supplied via `ROLTER_MASTER_KEY` (env/file). Pluggable backends (HashiCorp Vault, cloud KMS) are a roadmap item.
- In the **bootstrap file**, prefer `api_key_env` over inline `api_key` so secrets stay in the environment, not on disk.
- **Virtual keys** are stored as hashes with a short display prefix; the raw key is shown once at creation.
- Secrets are never logged. The gateway redacts auth headers from traces.

## Transport

- Upstream calls use rustls (no OpenSSL). HTTP/2 keep-alive with connection pooling.
- Optional per-provider **egress proxy** (`egress_proxy`, HTTP/HTTPS/SOCKS5) for networks where providers aren't directly reachable.
- Optional global or per-provider **custom CA bundles** add private PKI roots to outbound upstream clients while retaining public roots, certificate-chain validation, and hostname verification.
- Terminate TLS at the gateway or a fronting proxy/ingress in production.

## Egress policy (SSRF)

A provider's `api_base` decides where the gateway sends traffic, so an admin
surface that accepts an arbitrary one turns the proxy into an SSRF primitive
aimed at whatever the gateway's network position can reach.

rolter is built to run self-hosted and air-gapped, so upstreams legitimately
live on loopback, RFC1918 and container networks — blanket-denying private
destinations would break the core use case. What no legitimate LLM upstream
needs is the **link-local** range, which is where cloud instance metadata lives
(`169.254.169.254`, `fe80::/10`). That is denied by default; everything else is
opt-in:

```toml
[egress]
block_link_local = true   # default — cloud instance metadata
block_loopback   = false  # sidecar / single-host deployments
block_private    = false  # on-prem clusters
allow_hosts      = []     # exact-host escape hatch
```

Enforced in two places: the control plane rejects a denied `api_base` at
**write time** (`400`, so it never reaches the database), and snapshot
validation re-checks it, so a bootstrap toml can't smuggle one in either.

Config-time validation classifies **IP literals** only. Resolving hostnames
there would make validation depend on live DNS — reintroducing the "one bad row
freezes every gateway" failure mode described in
[config-and-hot-reload.md](config-and-hot-reload.md) — and would be bypassable
by DNS rebinding regardless.

So the same policy is enforced a third time, at **connect time**, by a custom
resolver on every upstream client: whatever DNS actually returns is classified
immediately before the connection is made. That covers the two cases config
validation cannot see — a hostname like `metadata.internal`, and a name that
resolved to a public address when it was configured but resolves to
`169.254.169.254` at request time. A denied destination surfaces as a policy
error naming the host, not an opaque connect failure.

A name that resolves to several addresses keeps the permitted ones: refusing
the whole name would take down a legitimate multi-homed upstream. Only a name
left with nothing is refused — which is exactly what rebinding to a denied
address produces. The resolver reads the policy from a live handle, so a hot
reload re-tunes enforcement without discarding pooled connections.

## Control-plane input validation

Every control-plane mutation body is decoded through a `SafeJson` extractor
rather than axum's `Json`. Before the body is deserialized into its typed
struct, every string in it — nested objects, arrays and object keys included —
is screened for control characters, and the request is rejected with a `400`
naming the offending field.

The concrete failure this prevents: Postgres `text` columns cannot store a NUL
byte, so a field carrying one failed deep inside the store and surfaced as an
unhandled `500` instead of input validation, violating the "bounded error, no
`unwrap`/`expect` on a request path" invariant. Screening the whole C0/C1 range
(and `U+007F`) also closes the log-injection vector a raw escape or newline
would otherwise open in operator-facing logs.

Tab, newline and carriage return stay allowed — multi-line values are
legitimate, a PEM CA bundle being the obvious one. Malformed JSON now also
comes back in the same OpenAI-style error envelope as every other failure
instead of axum's default rejection body.

## Control↔data-plane trust boundary

`GET /internal/snapshot` returns provider `api_key`s **decrypted**. That is by
necessity — the data plane needs the upstream credential to authenticate to the
provider — but it makes the snapshot channel a different trust boundary from
the operator-facing management API, and it should be configured as one:

```bash
ROLTER_INTERNAL_TOKEN=...            # gates /internal/*, distinct from ROLTER_ADMIN_TOKEN
ROLTER_INTERNAL_ADDR=127.0.0.1:4002  # serves /internal/* on its own socket
```

With `ROLTER_INTERNAL_TOKEN` set, the operator admin token no longer opens the
snapshot — only the gateway's own credential does. With `ROLTER_INTERNAL_ADDR`
set, `/internal/*` is not mounted on the public API router at all, so it is
absent from the port the dashboard and management API are served from rather
than merely gated on it. Bind it to loopback or a private interface. The
gateway sends `ROLTER_INTERNAL_TOKEN` on snapshot polls, falling back to
`ROLTER_ADMIN_TOKEN`.

Both are optional. Unset, the historical behavior holds — `/internal/*` shares
the public listener and accepts the admin token — and the control plane logs a
warning at startup saying so. The tenant-facing CRUD surface never returns a
provider key in any configuration.

Two things this deliberately does **not** do. There is no mTLS between the
planes: a shared secret over a private interface is comparable strength when
the network path is already trusted, and mTLS adds certificate lifecycle to an
air-gapped deployment. And the snapshot still carries plaintext rather than
sealed ciphertext: envelope-passing would require the gateway to hold the KEK,
moving the master secret onto every data-plane node, which is a worse trade for
most deployments. Revisit both if the planes ever cross an untrusted network.

## Wire transparency

- Outbound requests to upstream providers carry **no rolter-identifying marks**: no `User-Agent`, no added `X-*`/`Via` headers, no metadata injected into the JSON body, no marks in SSE framing. The only headers sent are functionally required ones — `content-type`, the provider's auth header, and `anthropic-version` for Anthropic.
- Responses back to clients likewise gain no rolter-added headers.
- This is a tested guarantee: golden wire tests in `rolter-proxy` capture the raw outbound request head and fail on any unexpected header (see `openai_wire_carries_no_rolter_signature`).

## Threat model (high level)

- **Tenant isolation**: virtual keys are scoped to a project; model allow-lists prevent access to unconfigured models; cache keys are namespaced to avoid cross-tenant cache poisoning.
- **Abuse**: RPM/TPM rate limits and budgets bound spend and load (roadmap enforcement).
- **AuthZ**: control-plane mutations are RBAC-checked and recorded in `audit_log`.
- **Supply chain**: `cargo deny`/advisory scanning in CI is a roadmap item.

## Operational guidance

- Always set a strong `ROLTER_MASTER_KEY` (e.g. `openssl rand -hex 32`) and rotate provider keys periodically.
- Run the control plane on a private network; expose only the gateway publicly.
- Back up Postgres; treat the master key as the most sensitive secret.
