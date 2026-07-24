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

Only **IP literals** are classified. Resolving hostnames during config
validation would make it depend on live DNS — reintroducing the "one bad row
freezes every gateway" failure mode described in
[config-and-hot-reload.md](config-and-hot-reload.md) — and would be bypassable
by DNS rebinding regardless. Connect-time address re-checking is tracked
separately.

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
