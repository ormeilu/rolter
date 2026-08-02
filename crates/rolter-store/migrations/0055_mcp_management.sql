-- Complete the organization-owned MCP registry used by the dashboard (#561).
-- Enabled servers are the only rows projected into gateway snapshots. Tool
-- groups and settings are control-plane policy definitions; the MCP proxy does
-- not yet use group membership as an authorization boundary.

alter table mcp_servers
    add column if not exists description text not null default '',
    add column if not exists enabled boolean not null default true,
    add column if not exists tools text[] not null default '{}',
    add column if not exists source text not null default 'custom'
        check (source in ('custom', 'library'));

create table if not exists mcp_tool_groups (
    id          uuid primary key default gen_random_uuid(),
    org_id      uuid not null references orgs (id) on delete cascade,
    name        text not null,
    slug        text not null,
    description text not null default '',
    enabled     boolean not null default true,
    tools       jsonb not null default '[]'::jsonb check (jsonb_typeof(tools) = 'array'),
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now(),
    unique (org_id, slug),
    constraint mcp_tool_groups_slug_charset check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$')
);

create table if not exists mcp_gateway_settings (
    org_id                 uuid primary key references orgs (id) on delete cascade,
    default_transport      text not null default 'streamable_http'
                           check (default_transport in ('sse', 'streamable_http', 'websocket')),
    connect_timeout_ms     integer not null default 5000 check (connect_timeout_ms between 100 and 60000),
    request_timeout_ms     integer not null default 30000 check (request_timeout_ms between 1000 and 300000),
    max_retries            integer not null default 1 check (max_retries between 0 and 5),
    default_failure_mode   text not null default 'fail_closed'
                           check (default_failure_mode in ('fail_open', 'fail_closed')),
    allow_unlisted_tools   boolean not null default false,
    updated_at             timestamptz not null default now()
);

-- Existing mcp_servers statement triggers already cover the new enabled field,
-- so toggling a server wakes snapshot polling. Groups and settings are not read
-- on the gateway request path yet and deliberately do not bump config_version.
