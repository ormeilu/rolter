-- Align the MCP transport allowlist across the control plane, the dashboard
-- and the schema (#783).
--
-- The set was defined in three places that disagreed: the control plane
-- accepted `stdio`, the dashboard's dropdown never offered it, and
-- `mcp_gateway_settings.default_transport` rejected it. An API-created stdio
-- server therefore rendered with an unknown transport badge and could not be
-- round-tripped through the Configure dialog.
--
-- `stdio` is dropped rather than added everywhere: it names a local subprocess,
-- which a hosted control plane cannot dial, and the gateway's HTTP proxy path
-- has never served it. `mcp_gateway_settings` already excluded it, so this
-- makes `mcp_servers` agree with the constraint that was already stricter.

-- Any stdio row that predates this is disabled rather than deleted: nothing
-- dials a disabled server, so retagging its transport cannot change what it
-- talks to, and an operator must review it before it can be enabled again.
-- Deleting would silently drop the OAuth grants and sessions that cascade
-- from it, which is a much larger blast radius than an operator expects from
-- a migration.
update mcp_servers
   set enabled   = false,
       transport = 'streamable_http'
 where transport = 'stdio';

alter table mcp_servers
    drop constraint if exists mcp_servers_transport_check;

alter table mcp_servers
    add constraint mcp_servers_transport_check
    check (transport in ('sse', 'streamable_http', 'websocket'));
