-- inventory of data-plane and control-plane nodes, refreshed from the snapshot
-- poll each node already makes so there is no second config channel (#543)
create table if not exists cluster_nodes (
    id             text primary key check (length(trim(id)) between 1 and 128),
    role           text not null check (role in ('gateway', 'control')),
    build_version  text not null default '',
    config_version bigint not null default 0,
    first_seen_at  timestamptz not null default now(),
    last_seen_at   timestamptz not null default now()
);

create index if not exists cluster_nodes_last_seen_idx
    on cluster_nodes (last_seen_at desc);
