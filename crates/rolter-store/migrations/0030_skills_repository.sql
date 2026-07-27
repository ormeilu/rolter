-- org-scoped skills repository with immutable versions
create table if not exists skills (
    id                uuid primary key default gen_random_uuid(),
    org_id            uuid not null references orgs(id) on delete cascade,
    name              text not null check (length(trim(name)) > 0),
    slug              text not null check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    description       text not null default '',
    retired_at        timestamptz,
    published_version integer check (published_version is null or published_version > 0),
    allowed_team_ids  uuid[] not null default '{}',
    minimum_role      text not null default 'viewer'
                      check (minimum_role in ('viewer', 'member', 'admin')),
    created_at        timestamptz not null default now(),
    unique (org_id, slug)
);

create table if not exists skill_versions (
    skill_id     uuid not null references skills(id) on delete cascade,
    version      integer not null check (version > 0),
    content      text,
    content_ref  text,
    metadata     jsonb not null default '{}'::jsonb,
    created_at   timestamptz not null default now(),
    primary key (skill_id, version),
    check (
        (content is not null and length(content) > 0 and content_ref is null)
        or
        (content is null and content_ref is not null and length(content_ref) > 0)
    ),
    check (jsonb_typeof(metadata) = 'object')
);

alter table skills
    add constraint skills_published_version_fk
    foreign key (id, published_version)
    references skill_versions(skill_id, version);
