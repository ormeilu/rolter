-- Outbound observability connectors (#511): where a deployment ships its
-- request logs, traces and metrics in addition to ClickHouse.
--
-- Strictly opt-in and off by default, because an air-gapped deployment must
-- keep working: a row with `enabled = false` sends nothing, and with no rows at
-- all there is no egress path.
--
-- Deliberately NOT given a bump_config_version() trigger. Connectors are
-- consumed by the control plane, not the data plane — the gateway never reads
-- this table, so bumping the version would wake the whole fleet for a change no
-- gateway can observe. See 0058, which records the same reasoning for the RBAC
-- tables.
create table if not exists observability_connectors (
    id                  uuid primary key default gen_random_uuid(),
    name                text not null unique,
    -- OTLP/HTTP first; datadog, prometheus remote-write and langfuse follow.
    -- a check constraint rather than an enum so widening it is one migration
    kind                text not null check (kind in ('otlp_http')),
    endpoint            text not null,
    enabled             boolean not null default false,
    -- 0.0 sends nothing, 1.0 sends everything; the head of the range is the
    -- expensive end, so the default is the cheap one
    sampling_rate       double precision not null default 1.0
                        check (sampling_rate >= 0.0 and sampling_rate <= 1.0),
    -- an external secret-manager reference, kept as an opaque string
    auth_secret_ref     text,
    -- or a managed secret, sealed with the deployment KEK before it lands here.
    -- write-only: never returned by the API, never placed in an audit detail
    auth_ciphertext     bytea,
    auth_nonce          bytea,
    -- last observed delivery health, set by the test-delivery endpoint
    health_status       text not null default 'unknown'
                        check (health_status in ('unknown', 'healthy', 'unhealthy')),
    health_checked_at   timestamptz,
    health_error        text,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now(),
    check ((auth_ciphertext is null) = (auth_nonce is null))
);

create index if not exists observability_connectors_enabled_idx
    on observability_connectors (enabled)
    where enabled;
