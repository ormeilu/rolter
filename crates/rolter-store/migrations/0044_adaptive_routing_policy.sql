-- singleton adaptive-routing policy projected into gateway snapshots (#544).
-- the defaults mirror AdaptiveRoutingConfig::default: off, so inserting this
-- row changes no routing behaviour until an operator enables it.
create table if not exists adaptive_routing_policy (
    id                boolean primary key default true check (id),
    enabled           boolean not null default false,
    latency_weight    real not null default 1.0 check (latency_weight between 0 and 100),
    cost_weight       real not null default 0.5 check (cost_weight between 0 and 100),
    load_weight       real not null default 0.25 check (load_weight between 0 and 100),
    exploration_ratio real not null default 0.05 check (exploration_ratio between 0 and 0.5),
    min_samples       integer not null default 50 check (min_samples between 0 and 1000000),
    -- at least one signal must carry weight, otherwise "adaptive" would be a
    -- random balancer; the gateway also refuses to engage in that state
    constraint adaptive_routing_policy_has_signal
        check (latency_weight > 0 or cost_weight > 0 or load_weight > 0),
    updated_at        timestamptz not null default now()
);

insert into adaptive_routing_policy (id) values (true) on conflict (id) do nothing;

drop trigger if exists adaptive_routing_policy_bump_config_version on adaptive_routing_policy;
create trigger adaptive_routing_policy_bump_config_version
after insert or update or delete on adaptive_routing_policy
for each statement execute function bump_config_version();
