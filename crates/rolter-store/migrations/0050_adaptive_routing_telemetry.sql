-- latest adaptive-routing state each gateway node reports for each route it
-- balances with the `adaptive` strategy (#751).
--
-- This is a scoreboard, not history: one row per (node, model), overwritten by
-- every report, so the table stays the size of the fleet rather than growing
-- with traffic. Time series, if it is ever wanted, belongs in the request log.
--
-- Deliberately NO bump_config_version() trigger. The data plane never reads
-- this table — it writes it — and a version bump on every report would make
-- each sample look like a config change and wake every gateway in the fleet.
create table if not exists adaptive_routing_telemetry (
    node_id           text not null check (length(trim(node_id)) between 1 and 128),
    model             text not null check (length(trim(model)) between 1 and 512),
    -- whether the blend was routing at sample time, rather than the fallback
    engaged           boolean not null default false,
    -- picks the route observed since the node built this balancer
    observed          bigint not null default 0,
    blend_picks       bigint not null default 0,
    exploration_picks bigint not null default 0,
    fallback_picks    bigint not null default 0,
    -- the sanitized policy the node actually applies, which can lag the stored
    -- one until that node converges on the config version carrying it
    policy            jsonb not null default '{}'::jsonb,
    -- per-target signals and scores, target-order aligned with the route
    targets           jsonb not null default '[]'::jsonb,
    reported_at       timestamptz not null default now(),
    primary key (node_id, model)
);

create index if not exists adaptive_routing_telemetry_reported_idx
    on adaptive_routing_telemetry (reported_at desc);
