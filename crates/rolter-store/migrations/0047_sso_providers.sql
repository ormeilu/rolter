-- OIDC single sign-on (#240): per-org identity providers, the group→role
-- mappings that turn an IdP claim into a rolter membership, and the short-lived
-- login states that make the authorization-code flow safe.

create table if not exists sso_providers (
    id                uuid primary key default gen_random_uuid(),
    org_id            uuid not null references orgs (id) on delete cascade,
    name              text not null,
    -- url-safe identity used in /auth/sso/{slug}/start
    slug              text not null,
    -- the issuer whose /.well-known/openid-configuration is discovered
    issuer            text not null,
    client_id         text not null,
    -- sealed with the deployment KEK, exactly like provider credentials
    secret_ciphertext bytea,
    secret_nonce      bytea,
    scopes            text[] not null default '{openid,email,profile}',
    -- id-token claim carrying the user's groups
    group_claim       text not null default 'groups',
    -- role granted at the org when no group mapping matches; null denies login
    -- to users the IdP has not put in a mapped group
    default_role      text check (default_role in ('admin', 'member', 'viewer')),
    enabled           boolean not null default true,
    created_at        timestamptz not null default now(),
    constraint sso_providers_org_slug_unique unique (org_id, slug),
    constraint sso_providers_slug_charset check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    constraint sso_providers_secret_pair check (
        (secret_ciphertext is null) = (secret_nonce is null)
    )
);

-- a group name from the IdP granting a role at an org/team/project scope, with
-- the same "most specific non-null id" convention as memberships
create table if not exists sso_group_mappings (
    id          uuid primary key default gen_random_uuid(),
    provider_id uuid not null references sso_providers (id) on delete cascade,
    group_name  text not null,
    org_id      uuid references orgs (id) on delete cascade,
    team_id     uuid references teams (id) on delete cascade,
    project_id  uuid references projects (id) on delete cascade,
    role        text not null check (role in ('admin', 'member', 'viewer')),
    created_at  timestamptz not null default now(),
    constraint sso_group_mappings_unique unique (provider_id, group_name, org_id, team_id, project_id)
);

create index if not exists sso_group_mappings_provider_idx
    on sso_group_mappings (provider_id, group_name);

-- one row per in-flight login. holds the PKCE verifier and the CSRF state; rows
-- are consumed on callback and swept by age, so a replayed callback finds
-- nothing to consume.
create table if not exists sso_login_states (
    state         text primary key,
    provider_id   uuid not null references sso_providers (id) on delete cascade,
    code_verifier text not null,
    nonce         text not null,
    redirect_uri  text not null,
    created_at    timestamptz not null default now()
);

create index if not exists sso_login_states_created_idx on sso_login_states (created_at);
