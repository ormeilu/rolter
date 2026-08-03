-- token acquisition for MCP OAuth (#707). 0045 gave grants and sessions a
-- home but no way to mint one: rows could only be written directly. this adds
-- the OAuth client registration a server needs before it can be consented to,
-- and the one-shot login states that make the authorization-code exchange safe.

-- the OAuth client rolter presents to an MCP server's authorization server.
-- the secret is sealed with the deployment KEK exactly like provider_keys and
-- sso_providers; there is deliberately no plaintext column.
alter table mcp_servers
    add column if not exists authorize_url            text,
    add column if not exists token_url                text,
    add column if not exists client_id                text,
    add column if not exists client_secret_ciphertext bytea,
    add column if not exists client_secret_nonce      bytea,
    add column if not exists default_scopes           text[] not null default '{}';

do $$
begin
    alter table mcp_servers
        add constraint mcp_servers_client_secret_pair check (
            (client_secret_ciphertext is null) = (client_secret_nonce is null)
        );
exception
    when duplicate_object then null;
end
$$;

-- one row per in-flight consent. holds the sealed PKCE verifier and the CSRF
-- state; the row is consumed by the callback, so a replayed code+state pair
-- finds nothing to redeem, and stale rows are swept by age.
create table if not exists mcp_oauth_login_states (
    state               text primary key,
    server_id           uuid not null references mcp_servers (id) on delete cascade,
    user_id             uuid not null references users (id) on delete cascade,
    -- the PKCE verifier is proof-of-possession for the authorization code, so
    -- it is sealed rather than stored in the clear
    verifier_ciphertext bytea not null,
    verifier_nonce      bytea not null,
    scopes              text[] not null default '{}',
    redirect_uri        text not null,
    created_at          timestamptz not null default now()
);

create index if not exists mcp_oauth_login_states_created_idx
    on mcp_oauth_login_states (created_at);

-- the refresher sweeps live sessions by access expiry; without this it is a
-- table scan on every tick
create index if not exists mcp_oauth_sessions_expiry_idx
    on mcp_oauth_sessions (expires_at)
    where revoked_at is null;
