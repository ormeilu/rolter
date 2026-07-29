# MCP servers, OAuth grants and sessions

rolter stores the OAuth state behind Model Context Protocol access: which MCP servers an org has registered, which users consented to what, and the token sessions held against those consents. Transport and the authorization-code / on-behalf-of exchange belong to the MCP proxy; this layer owns persistence, listing, revocation and audit, so it is useful to a stdio, SSE, streamable-HTTP or WebSocket implementation alike.

## Model

```
mcp_servers (org-scoped)
   └── mcp_oauth_grants   one live grant per (server, user)
          └── mcp_oauth_sessions   token material, sealed at rest
```

- **Server** — name, slug (unique per org), URL and transport. Deleting one cascades to its grants and sessions: withdrawing the server withdraws access to it.
- **Grant** — a user's consent against a server, with the scope set they agreed to. A user holds at most one *live* grant per server (a partial unique index on `revoked_at is null`); revoked grants are kept so the audit trail survives. Re-consenting updates the scopes in place rather than accumulating rows.
- **Session** — the tokens issued under a grant, with `expires_at`, an optional refresh token and `refresh_expires_at`.

## Token handling

Access and refresh tokens are sealed with AES-256-GCM under the deployment KEK (`ROLTER_KEK`), the same mechanism as upstream provider credentials — there is deliberately no plaintext column for either. Ciphertext and nonce sit side by side; the KEK never reaches the database.

The store exposes exactly one way to read them back, `McpOAuthRepo::open_session`, which the MCP proxy calls on the request path. It returns `None` unless the session is unrevoked, unexpired **and** its grant is still live, so a withdrawn consent cannot be spent. Every other repository method returns metadata only, and the session DTO carries a `has_refresh_token` boolean instead of the token — a UI can show renewability without the control plane ever handing a token out.

## Who sees what

| caller | grants / sessions visible | may revoke |
| --- | --- | --- |
| superadmin / admin token | every one in the org | any |
| org admin | every one in the org | any |
| org member or viewer | only the ones they own | only their own |
| anyone outside the org | none (`403`) | none (`403`) |

Every listing is joined through `mcp_servers.org_id`, so a cross-tenant read is not expressible, not merely filtered out.

## Endpoints

- `GET`/`POST /api/v1/orgs/{org_id}/mcp-servers`, `DELETE /api/v1/mcp-servers/{id}` — viewer reads, admin writes. A server URL must be `http(s)`; the transport must be one of `stdio`, `sse`, `streamable_http`, `websocket`.
- `GET /api/v1/orgs/{org_id}/mcp/grants`, `DELETE /api/v1/mcp/grants/{id}`
- `GET /api/v1/orgs/{org_id}/mcp/sessions`, `DELETE /api/v1/mcp/sessions/{id}`

Revoking a grant revokes every session under it **in the same transaction**, so consent and tokens can never disagree. Server creation/deletion and both revocations are written to `audit_log`.
