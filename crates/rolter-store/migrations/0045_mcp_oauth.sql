-- MCP servers plus the OAuth consent grants and token sessions held against
-- them (#541). token material is sealed with the same AES-256-GCM KEK as
-- provider_keys and never leaves the store layer in plaintext.

create table if not exists mcp_servers (
    id          uuid primary key default gen_random_uuid(),
    org_id      uuid not null references orgs (id) on delete cascade,
    name        text not null,
    slug        text not null,
    url         text not null,
    transport   text not null default 'streamable_http'
                check (transport in ('stdio', 'sse', 'streamable_http', 'websocket')),
    created_at  timestamptz not null default now(),
    constraint mcp_servers_org_slug_unique unique (org_id, slug),
    constraint mcp_servers_slug_charset check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$')
);

-- one consent grant per (server, user). scopes are what the user consented to;
-- a session may never hold more than these.
create table if not exists mcp_oauth_grants (
    id          uuid primary key default gen_random_uuid(),
    server_id   uuid not null references mcp_servers (id) on delete cascade,
    user_id     uuid not null references users (id) on delete cascade,
    scopes      text[] not null default '{}',
    granted_at  timestamptz not null default now(),
    revoked_at  timestamptz,
    revoked_by  uuid references users (id) on delete set null
);

-- a user may hold at most one live grant per server; revoked ones are kept for
-- the audit trail, so the uniqueness is partial
create unique index if not exists mcp_oauth_grants_active_unique
    on mcp_oauth_grants (server_id, user_id)
    where revoked_at is null;

create index if not exists mcp_oauth_grants_user_idx on mcp_oauth_grants (user_id);

-- token material for a grant. access/refresh tokens are stored as ciphertext +
-- nonce; there is deliberately no plaintext column for either.
create table if not exists mcp_oauth_sessions (
    id                 uuid primary key default gen_random_uuid(),
    grant_id           uuid not null references mcp_oauth_grants (id) on delete cascade,
    access_ciphertext  bytea not null,
    access_nonce       bytea not null,
    refresh_ciphertext bytea,
    refresh_nonce      bytea,
    scopes             text[] not null default '{}',
    expires_at         timestamptz not null,
    refresh_expires_at timestamptz,
    revoked_at         timestamptz,
    created_at         timestamptz not null default now(),
    last_used_at       timestamptz,
    -- a refresh token is either fully present or fully absent
    constraint mcp_oauth_sessions_refresh_pair check (
        (refresh_ciphertext is null) = (refresh_nonce is null)
    )
);

create index if not exists mcp_oauth_sessions_grant_idx on mcp_oauth_sessions (grant_id);
