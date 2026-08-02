-- control-plane registry for configured plugin instances; gateway dispatch is
-- tracked separately in #509, so these rows intentionally do not bump the
-- gateway config version yet
create table plugin_instances (
    id              uuid primary key default gen_random_uuid(),
    org_id          uuid not null references orgs(id) on delete cascade,
    project_id      uuid references projects(id) on delete cascade,
    name            text not null check (length(trim(name)) > 0),
    slug            text not null check (slug ~ '^[a-z0-9][a-z0-9_-]*$'),
    description     text not null default '',
    kind            text not null default 'webhook' check (kind in ('webhook')),
    stage           text not null check (stage in ('pre_route', 'pre_upstream', 'post_response')),
    enabled         boolean not null default false,
    position        integer not null default 0 check (position >= 0),
    failure_mode    text not null default 'fail_open' check (failure_mode in ('fail_open', 'fail_closed')),
    endpoint        text not null check (endpoint ~ '^https?://'),
    secret_env      text check (secret_env is null or length(trim(secret_env)) > 0),
    config          jsonb not null default '{}'::jsonb check (jsonb_typeof(config) = 'object'),
    created_at      timestamptz not null default now(),
    updated_at      timestamptz not null default now()
);

create unique index plugin_instances_org_slug
    on plugin_instances (org_id, slug)
    where project_id is null;

create unique index plugin_instances_project_slug
    on plugin_instances (org_id, project_id, slug)
    where project_id is not null;
