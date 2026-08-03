-- Configurable RBAC: org-scoped custom roles, explicit resource/action grants,
-- and reusable access profiles composed from them (#534).
--
-- The built-in admin/member/viewer roles are untouched. Everything here is
-- **additive**: a custom grant can only widen what a membership already gives,
-- never narrow it, so a deployment with no rows in these tables behaves exactly
-- as it did before the migration. That is the whole compatibility contract, and
-- it is the reason the resolver keeps `memberships` as its primary input and
-- consults these tables only after the membership check has already failed.
--
-- None of these tables carries a `bump_config_version()` trigger, and none may
-- grow one: the data plane does not consume them. Control-plane authorization
-- is evaluated per request against the live database, so there is nothing to
-- propagate through `/internal/snapshot`, and bumping the version on every role
-- edit would wake the whole gateway fleet for a change it cannot observe.

-- a named role an operator defines inside one org. `base_role` is the built-in
-- role the custom role is at least equivalent to, which is what lets a profile
-- express "these people are admins here" without re-enumerating every grant.
create table if not exists custom_roles (
    id          uuid primary key default gen_random_uuid(),
    org_id      uuid not null references orgs (id) on delete cascade,
    -- url-safe identity, stable across renames
    slug        text not null,
    name        text not null,
    description text,
    -- floor this role grants at the scope it is composed in; `viewer` is the
    -- safe default because it adds nothing an authenticated org member of any
    -- kind does not already have
    base_role   text not null default 'viewer'
                check (base_role in ('admin', 'member', 'viewer')),
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now(),
    constraint custom_roles_org_slug_unique unique (org_id, slug),
    constraint custom_roles_slug_charset check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$')
);

-- one explicit (resource, action) grant on a custom role. `resource` is a row
-- in the control plane's capability table (rbac_matrix.rs) and is deliberately
-- not a foreign key: the table is code, not data, and a role naming a resource
-- a future build removed must degrade to "grants nothing" rather than block the
-- migration that removed it.
create table if not exists custom_role_grants (
    id       uuid primary key default gen_random_uuid(),
    role_id  uuid not null references custom_roles (id) on delete cascade,
    resource text not null,
    action   text not null check (action in ('read', 'create', 'update', 'delete')),
    constraint custom_role_grants_unique unique (role_id, resource, action)
);

create index if not exists custom_role_grants_role_idx on custom_role_grants (role_id);

-- a reusable bundle: a name an operator assigns to people, rather than a role
-- they assign one scope at a time
create table if not exists access_profiles (
    id          uuid primary key default gen_random_uuid(),
    org_id      uuid not null references orgs (id) on delete cascade,
    slug        text not null,
    name        text not null,
    description text,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now(),
    constraint access_profiles_org_slug_unique unique (org_id, slug),
    constraint access_profiles_slug_charset check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$')
);

-- the composition: which custom role the profile confers, at which scope. The
-- scope follows the same "most specific non-null id" convention as memberships
-- and sso_group_mappings, so one resolver understands all three.
--
-- `on delete restrict` on the role is the deletion rule (#534): a custom role
-- still composed into a profile cannot be deleted, because the alternative —
-- silently dropping the grant, or silently falling back to the base role —
-- changes what people can do with nothing in the audit log naming the profile
-- it happened to. Detaching is its own audited step.
create table if not exists access_profile_roles (
    id         uuid primary key default gen_random_uuid(),
    profile_id uuid not null references access_profiles (id) on delete cascade,
    role_id    uuid not null references custom_roles (id) on delete restrict,
    org_id     uuid references orgs (id) on delete cascade,
    team_id    uuid references teams (id) on delete cascade,
    project_id uuid references projects (id) on delete cascade,
    created_at timestamptz not null default now(),
    constraint access_profile_roles_unique
        unique (profile_id, role_id, org_id, team_id, project_id),
    -- a composition with no scope at all would apply nowhere; reject it here
    -- rather than store a row that can never match a chain
    constraint access_profile_roles_scoped
        check (org_id is not null or team_id is not null or project_id is not null)
);

create index if not exists access_profile_roles_profile_idx on access_profile_roles (profile_id);
create index if not exists access_profile_roles_role_idx on access_profile_roles (role_id);

-- who holds the profile: exactly one of a user or a team. A team assignment
-- reaches every member of that team, including via a project inside it.
create table if not exists access_profile_assignments (
    id         uuid primary key default gen_random_uuid(),
    profile_id uuid not null references access_profiles (id) on delete cascade,
    user_id    uuid references users (id) on delete cascade,
    team_id    uuid references teams (id) on delete cascade,
    created_at timestamptz not null default now(),
    constraint access_profile_assignments_subject
        check ((user_id is null) <> (team_id is null))
);

-- one assignment per (profile, subject); two partial indexes because a plain
-- unique constraint treats every null as distinct
create unique index if not exists access_profile_assignments_user_unique
    on access_profile_assignments (profile_id, user_id) where user_id is not null;
create unique index if not exists access_profile_assignments_team_unique
    on access_profile_assignments (profile_id, team_id) where team_id is not null;
create index if not exists access_profile_assignments_user_idx
    on access_profile_assignments (user_id) where user_id is not null;
create index if not exists access_profile_assignments_team_idx
    on access_profile_assignments (team_id) where team_id is not null;

-- model/route visibility carried by a profile. Empty allow-list means "no
-- restriction", so an existing deployment (which has no profiles at all) is
-- unrestricted, and adding a profile without a policy row stays unrestricted.
-- Entries are exact names or a trailing-`*` prefix glob; deny beats allow.
create table if not exists access_profile_policies (
    profile_id     uuid primary key references access_profiles (id) on delete cascade,
    allowed_models text[] not null default '{}',
    denied_models  text[] not null default '{}',
    allowed_routes text[] not null default '{}',
    denied_routes  text[] not null default '{}',
    updated_at     timestamptz not null default now()
);
