-- Project MCP server policy and live OAuth sessions into gateway snapshots.
-- Every source-table change must wake snapshot polling so revocation and expiry
-- policy changes cannot leave a gateway serving stale authorization state.

alter table mcp_servers
    add column if not exists required_scopes text[] not null default '{}';

drop trigger if exists mcp_servers_bump_config_version on mcp_servers;
create trigger mcp_servers_bump_config_version
after insert or update or delete on mcp_servers
for each statement execute function bump_config_version();

drop trigger if exists mcp_oauth_grants_bump_config_version on mcp_oauth_grants;
create trigger mcp_oauth_grants_bump_config_version
after insert or update or delete on mcp_oauth_grants
for each statement execute function bump_config_version();

drop trigger if exists mcp_oauth_sessions_bump_config_version on mcp_oauth_sessions;
create trigger mcp_oauth_sessions_bump_config_version
after insert or delete on mcp_oauth_sessions
for each statement execute function bump_config_version();

-- last_used_at is request bookkeeping, not authorization state. Keep it out
-- of config propagation so touching a session cannot wake every gateway.
drop trigger if exists mcp_oauth_sessions_auth_bump_config_version on mcp_oauth_sessions;
create trigger mcp_oauth_sessions_auth_bump_config_version
after update of grant_id, access_ciphertext, access_nonce, scopes, expires_at, revoked_at
on mcp_oauth_sessions
for each statement execute function bump_config_version();
