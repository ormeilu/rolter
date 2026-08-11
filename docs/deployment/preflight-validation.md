# Pre-boot validation (`rolter check`)

rolter has two deployment paths, and they are deliberately not converging.

**Local** (`rolter easy-up`) is frictionless and may stay loose on security: no
database, no provider keys, no secrets at all, answering immediately on the
built-in `fake-llm` model. It prompts for nothing, and `rolter check` does not
change that.

**Production** is guided and should be hard to misconfigure. `rolter check`
validates a production deployment *before* the process starts, so a
misconfiguration fails loudly instead of starting degraded.

```bash
rolter check                      # validate the current environment
rolter check --config rolter.toml # also parse and validate the config file
rolter check --strict             # treat warnings as failures too
```

It exits non-zero when anything fatal is found, which makes it usable as a
container entrypoint step, a Kubernetes init container, or a Helm pre-install
hook.

## Why this exists

The motivating case is worth stating plainly, because it is the shape of every
problem this command is meant to catch.

Provider credentials are encrypted at rest with a key-encryption key read from
`ROLTER_KEK`. When that variable is missing, the control plane's
default-provider seed does not fail — it logs a warning and stores the inline
`api_key` **unsealed**. Nothing goes red. The deployment serves traffic
normally. A credential the operator believed was encrypted at rest simply is
not, and there is no signal saying so.

Until recently several docs pages — including the production deployment guide
and `.env.example` — told operators to set `ROLTER_MASTER_KEY`, a variable
nothing in the codebase has ever read. An operator who followed those
instructions exactly ended up in precisely that degraded state. Those pages are
now corrected, and `rolter check` reports the mistake by name if the old
variable is still set anywhere.

That is the class of failure this gate targets: not a crash, but a deployment
that looks healthy while quietly not doing what it was configured to do.

## What it checks

| Check | Severity | Why |
|---|---|---|
| `ROLTER_KEK` present | error | Without it provider credentials are stored unsealed, with only a warning |
| `ROLTER_KEK` length ≥ 16 | error | The KEK is stretched through SHA-256, so a weak secret still yields a valid — and brute-forceable — key |
| `ROLTER_ADMIN_TOKEN` present | error | Otherwise the management API and `/internal/snapshot` are unauthenticated |
| `ROLTER_DATABASE_URL` present and `postgres://` | error | Otherwise the control plane falls back to an in-memory store and loses all config on restart |
| No example values survive | error | The example database credentials and the e2e throwaway KEK are published in the repository |
| `ROLTER_REDIS_URL` present | warning | Without it rate limits and budgets are enforced per replica, not per deployment |
| Control plane not bound to `0.0.0.0` | warning | The management API should not be reachable on every interface |
| Config file parses (with `--config`) | error | A config that fails to load leaves the gateway on whatever it last had |

Redis and the bind address are warnings rather than errors because a
single-replica deployment behind an ingress is legitimately fine without either.
Use `--strict` in an environment where they are not.

Anything resembling credentials in a URL is redacted before printing, since this
output routinely lands in CI logs.

## In a container

Run it as a pre-start step so the container never reaches a serving state while
misconfigured:

```dockerfile
CMD ["sh", "-c", "rolter check --strict && rolter control"]
```

In Kubernetes, prefer an init container so the failure is visible as a distinct
pod status rather than a crash-looping main container:

```yaml
initContainers:
  - name: preflight
    image: ghcr.io/rolter-ai/rolter:latest
    command: ["rolter", "check", "--strict"]
    envFrom:
      - secretRef:
          name: rolter-secrets
```
