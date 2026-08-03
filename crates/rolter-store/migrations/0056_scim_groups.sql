-- SCIM 2.0 Groups (#540): the group half of provisioning, plus the org-scoped
-- group→team mapping that turns IdP group membership into rolter roles.

create table if not exists scim_groups (
    id           uuid primary key default gen_random_uuid(),
    org_id       uuid not null references orgs (id) on delete cascade,
    -- the IdP's stable id for the group; absent for IdPs that omit it
    external_id  text,
    -- SCIM displayName, unique within the org. mappings are written against
    -- this name, so an operator maps the group they see in the IdP UI.
    display_name text not null,
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),
    constraint scim_groups_org_display_name_unique unique (org_id, display_name)
);

create unique index if not exists scim_groups_org_external_id_unique
    on scim_groups (org_id, external_id)
    where external_id is not null;

-- group membership as the IdP sees it. the (group, user) pair is the whole row,
-- so an IdP re-sending the same member list converges instead of accumulating.
create table if not exists scim_group_members (
    group_id   uuid not null references scim_groups (id) on delete cascade,
    user_id    uuid not null references users (id) on delete cascade,
    created_at timestamptz not null default now(),
    primary key (group_id, user_id)
);

create index if not exists scim_group_members_user_idx on scim_group_members (user_id);

-- a SCIM group name granting a role at a team, a project, or the org itself,
-- with the same "most specific non-null id" convention as memberships and
-- sso_group_mappings. org-scoped rather than provider-scoped: a provisioning
-- token already resolves to exactly one org, and there is no provider row to
-- hang the mapping off.
create table if not exists scim_group_mappings (
    id         uuid primary key default gen_random_uuid(),
    org_id     uuid not null references orgs (id) on delete cascade,
    group_name text not null,
    team_id    uuid references teams (id) on delete cascade,
    project_id uuid references projects (id) on delete cascade,
    role       text not null check (role in ('admin', 'member', 'viewer')),
    created_at timestamptz not null default now(),
    constraint scim_group_mappings_unique unique (org_id, group_name, team_id, project_id)
);

-- the constraint above cannot see two org-scoped rows as equal, because
-- postgres treats nulls as distinct in a unique constraint. this closes that
-- hole for the one shape where both scope ids are null.
create unique index if not exists scim_group_mappings_org_scope_unique
    on scim_group_mappings (org_id, group_name, role)
    where team_id is null and project_id is null;

create index if not exists scim_group_mappings_org_group_idx
    on scim_group_mappings (org_id, group_name);

-- a membership a SCIM group mapping produced is reconciled on every group
-- change, exactly like an 'sso' row. 'manual' grants still survive untouched,
-- so an operator's own grant is never undone by a sync.
alter table memberships drop constraint if exists memberships_source_check;

alter table memberships
    add constraint memberships_source_check check (source in ('manual', 'sso', 'scim'));
