# SCIM 2.0 provisioning

An identity provider can create, update, deactivate and reconcile rolter accounts over SCIM 2.0 instead of an operator doing it by hand. This page covers the Users half; Groups (and group→team mapping) land in a later slice.

## Tokens carry the tenant

There is no org id anywhere in the SCIM path. An operator mints an org-scoped provisioning token, and that token is what resolves the tenant:

```
POST   /api/v1/orgs/{org_id}/scim-tokens   { "name": "okta" }   # org admin
GET    /api/v1/orgs/{org_id}/scim-tokens
DELETE /api/v1/scim-tokens/{id}
```

The plaintext token (`rolter_scim_…`) is returned **once**, at creation. Only its peppered SHA-256 digest is stored — the same treatment as session tokens — so a database dump alone cannot be replayed, and a lost token is rotated rather than looked up. Revoking takes effect on the very next request; there is no cache to invalidate.

Because the token resolves to exactly one org, an IdP cannot address another tenant's users: a foreign resource id is a `404`, not a redacted row.

## Users

```
GET    /scim/v2/Users?filter=userName eq "ada@example.com"
POST   /scim/v2/Users
GET    /scim/v2/Users/{id}
PUT    /scim/v2/Users/{id}
PATCH  /scim/v2/Users/{id}
DELETE /scim/v2/Users/{id}
```

- **Filters.** Only `userName eq "value"` is supported — the one shape reconciliation needs. Any other filter is a `400` with `scimType: invalidFilter` rather than being ignored: silently returning the whole directory reads to an IdP as "no such user", and it then re-creates the account.
- **Idempotence.** IdPs retry. Creating an existing `userName` returns `409` with `scimType: uniqueness` instead of a second account. If the email already belongs to a local account (invited by an admin, or provisioned elsewhere), that account is adopted rather than failing on the unique email, so provisioning converges.
- **PATCH.** The `active` toggle is implemented, in both the `path: "active"` and bare `{"active": false}` forms, and with the string `"true"`/`"false"` some IdPs send. Any other operation is a `400` — an IdP that gets a success for an operation nothing applied would believe the change landed.
- **Deactivation logs the user out.** Setting `active: false` stamps `deactivated_at` *and* deletes the account's live sessions, because an IdP disabling a leaver expects them out now, not merely unable to log in again.
- **DELETE deprovisions.** The account is deactivated, its sessions dropped, and the org's SCIM identity mapping removed. The `users` row itself stays: its memberships and audit trail must outlive any one IdP, and a later `POST` adopts it again.

## No password path

A `password` attribute in a SCIM body is ignored, not honoured. Provisioned accounts are SSO-shaped: `password_hash` stays null, and there is no code path through which SCIM can set or read a local credential.

## Roles

A provisioned account is granted a **viewer** membership at the org it was provisioned into — least privilege on purpose. SCIM decides *who exists*; an operator still decides what they may do. Group-driven role mapping arrives with the Groups slice.

## Audit

`scim.user.create`, `scim.user.update` and `scim.user.deprovision` are written to `audit_log`, scoped to the org, with the provisioning token's id in the detail — the actor is a token, not a human. Token creation and revocation are audited as `scim_token.create` / `scim_token.revoke` with the acting operator.
