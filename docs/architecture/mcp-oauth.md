# MCP servers, OAuth grants and sessions

rolter stores the OAuth state behind Model Context Protocol access: which MCP servers an org has registered, which users consented to what, and the token sessions held against those consents. Transport and the authorization-code / on-behalf-of exchange belong to the MCP proxy; this layer owns persistence, listing, revocation and audit, so it is useful to a stdio, SSE, streamable-HTTP or WebSocket implementation alike.

## Model

```
mcp_servers (org-scoped)
   └── mcp_oauth_grants   one live grant per (server, user)
          └── mcp_oauth_sessions   token material, sealed at rest
```

- **Server** — name, slug (unique per org), URL, transport and the OAuth scopes every proxied call requires. Deleting one cascades to its grants and sessions: withdrawing the server withdraws access to it.
- **Grant** — a user's consent against a server, with the scope set they agreed to. A user holds at most one *live* grant per server (a partial unique index on `revoked_at is null`); revoked grants are kept so the audit trail survives. Re-consenting updates the scopes in place rather than accumulating rows.
- **Session** — the tokens issued under a grant, with `expires_at`, an optional refresh token and `refresh_expires_at`.

## Token handling

Access and refresh tokens are sealed with AES-256-GCM under the deployment KEK (`ROLTER_KEK`), the same mechanism as upstream provider credentials — there is deliberately no plaintext column for either. Ciphertext and nonce sit side by side; the KEK never reaches the database.

The store exposes tokens through two credential-only paths. `McpOAuthRepo::open_session` opens one explicitly selected live session for lifecycle code. The Postgres config store opens only the newest live session per `(server, user)` whose scopes remain within its live grant and cover the server's required scopes; those records travel through the token-guarded `/internal/snapshot` channel already used for decrypted provider credentials. Public API DTOs carry only metadata and a `has_refresh_token` boolean.

The gateway indexes servers by `(org, slug)` and sessions by `(server, user)`. A request to `/mcp/{server}` must authenticate with a database-backed virtual key whose `created_by` user owns the selected session. The gateway repeats the required-scope check before connecting and replaces the caller's virtual key with the downstream bearer token. Revocation, expiry and policy writes bump `config_version`, so snapshot polling removes authorization without a restart.

## Who sees what

| caller | grants / sessions visible | may revoke |
| --- | --- | --- |
| superadmin / admin token | every one in the org | any |
| org admin | every one in the org | any |
| org member or viewer | only the ones they own | only their own |
| anyone outside the org | none (`403`) | none (`403`) |

Every listing is joined through `mcp_servers.org_id`, so a cross-tenant read is not expressible, not merely filtered out.

## Endpoints

- `GET`/`POST /api/v1/orgs/{org_id}/mcp-servers`, `PATCH`/`DELETE /api/v1/mcp-servers/{id}` — viewer reads, admin writes. A server URL must be `http(s)`; the transport must be one of `stdio`, `sse`, `streamable_http`, `websocket`. `PATCH` replaces `required_scopes` without destroying grants or sessions.
- `GET /api/v1/orgs/{org_id}/mcp/grants`, `DELETE /api/v1/mcp/grants/{id}`
- `GET /api/v1/orgs/{org_id}/mcp/sessions`, `DELETE /api/v1/mcp/sessions/{id}`
- `GET`/`POST`/`DELETE /mcp/{server_slug}/{path...}` — Streamable HTTP/SSE proxy on the gateway, authorized by virtual-key owner, server and required scopes

Revoking a grant revokes every session under it **in the same transaction**, so consent and tokens can never disagree. Server creation/deletion and both revocations are written to `audit_log`.
