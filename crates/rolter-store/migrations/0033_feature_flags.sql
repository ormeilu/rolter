-- singleton typed feature flags for control-plane managed subsystem gating
create table if not exists feature_flags (
    id                      boolean primary key default true check (id),
    response_cache          boolean not null default true,
    cache_aware_routing     boolean not null default true,
    circuit_breaker         boolean not null default true,
    active_health_checks    boolean not null default true,
    complexity_routing      boolean not null default true,
    guardrails              boolean not null default true,
    updated_at              timestamptz not null default now()
);

insert into feature_flags (id) values (true) on conflict (id) do nothing;

drop trigger if exists feature_flags_bump_config_version on feature_flags;
create trigger feature_flags_bump_config_version
after insert or update or delete on feature_flags
for each statement execute function bump_config_version();
