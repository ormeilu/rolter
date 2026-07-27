-- store-backed prompt templates with immutable versions and scope bindings
create table if not exists prompt_templates (
    id                uuid primary key default gen_random_uuid(),
    org_id            uuid not null references orgs(id) on delete cascade,
    name              text not null check (length(trim(name)) > 0),
    slug              text not null check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    description       text not null default '',
    published_version integer check (published_version is null or published_version > 0),
    created_at        timestamptz not null default now(),
    unique (org_id, slug)
);

create table if not exists prompt_template_versions (
    template_id uuid not null references prompt_templates(id) on delete cascade,
    version     integer not null check (version > 0),
    variables   jsonb not null default '[]'::jsonb,
    decorators  jsonb not null default '[]'::jsonb,
    created_at  timestamptz not null default now(),
    primary key (template_id, version),
    check (jsonb_typeof(variables) = 'array'),
    check (jsonb_typeof(decorators) = 'array')
);

alter table prompt_templates
    add constraint prompt_templates_published_version_fk
    foreign key (id, published_version)
    references prompt_template_versions(template_id, version);

create table if not exists prompt_template_scopes (
    template_id uuid not null,
    version     integer not null,
    scope_type  text not null check (scope_type in ('org', 'project', 'route', 'virtual_key')),
    scope_id    uuid not null,
    org_id      uuid references orgs(id) on delete cascade,
    project_id  uuid references projects(id) on delete cascade,
    route_id    uuid references routes(id) on delete cascade,
    virtual_key_id uuid references virtual_keys(id) on delete cascade,
    created_at  timestamptz not null default now(),
    primary key (template_id, version, scope_type, scope_id),
    foreign key (template_id, version)
        references prompt_template_versions(template_id, version)
        on delete cascade,
    check (
        (scope_type = 'org' and org_id = scope_id and project_id is null and route_id is null and virtual_key_id is null)
        or
        (scope_type = 'project' and org_id is null and project_id = scope_id and route_id is null and virtual_key_id is null)
        or
        (scope_type = 'route' and org_id is null and project_id is null and route_id = scope_id and virtual_key_id is null)
        or
        (scope_type = 'virtual_key' and org_id is null and project_id is null and route_id is null and virtual_key_id = scope_id)
    )
);

create or replace function reject_published_prompt_template_scope_change()
returns trigger language plpgsql as $$
declare
    target_template_id uuid := coalesce(new.template_id, old.template_id);
    target_version integer := coalesce(new.version, old.version);
begin
    if exists (
        select 1
        from prompt_templates
        where id = target_template_id
          and published_version = target_version
    ) then
        raise exception 'published prompt template scopes are immutable';
    end if;
    if tg_op = 'DELETE' then
        return old;
    end if;
    return new;
end;
$$;

create trigger prompt_template_published_scopes_immutable
    before insert or update or delete on prompt_template_scopes
    for each row execute function reject_published_prompt_template_scope_change();
