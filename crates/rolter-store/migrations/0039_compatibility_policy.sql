-- singleton cross-dialect compatibility policy projected into gateway
-- snapshots; both values were compiled-in constants before (#546)
create table if not exists compatibility_policy (
    id                 boolean primary key default true check (id),
    anthropic_version  text not null default '2023-06-01'
        check (anthropic_version ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'),
    default_max_tokens integer not null default 1024
        check (default_max_tokens between 1 and 1000000),
    updated_at         timestamptz not null default now()
);

insert into compatibility_policy (id) values (true) on conflict (id) do nothing;

drop trigger if exists compatibility_policy_bump_config_version on compatibility_policy;
create trigger compatibility_policy_bump_config_version
after insert or update or delete on compatibility_policy
for each statement execute function bump_config_version();
