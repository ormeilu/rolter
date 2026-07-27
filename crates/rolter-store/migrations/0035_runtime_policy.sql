-- singleton runtime policy projected into gateway snapshots for hot-reload
create table if not exists runtime_policy (
    id                 boolean primary key default true check (id),
    retry_max_retries  integer not null default 2 check (retry_max_retries between 0 and 10),
    retry_base_ms      integer not null default 100 check (retry_base_ms between 0 and 60000),
    retry_max_ms       integer not null default 2000 check (retry_max_ms between retry_base_ms and 600000),
    timeout_connect_s  integer not null default 10 check (timeout_connect_s between 0 and 300),
    timeout_request_s  integer not null default 60 check (timeout_request_s between 0 and 3600),
    queue_enabled      boolean not null default true,
    queue_capacity     integer not null default 256 check (queue_capacity between 1 and 100000),
    queue_workers      integer not null default 8 check (queue_workers between 1 and 2048),
    queue_backpressure text not null default 'error' check (queue_backpressure in ('drop', 'block', 'error')),
    queue_block_ms     integer not null default 1000 check (
        queue_block_ms between 0 and 120000
        and (queue_backpressure <> 'block' or queue_block_ms > 0)
    ),
    updated_at         timestamptz not null default now()
);

insert into runtime_policy (id) values (true) on conflict (id) do nothing;

drop trigger if exists runtime_policy_bump_config_version on runtime_policy;
create trigger runtime_policy_bump_config_version
after insert or update or delete on runtime_policy
for each statement execute function bump_config_version();
