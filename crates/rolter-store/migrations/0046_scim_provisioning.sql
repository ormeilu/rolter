-- SCIM 2.0 provisioning (#540): bearer tokens an IdP authenticates with, and
-- the org-scoped identity mapping between a SCIM resource and a local user.

create table if not exists scim_tokens (
    id           uuid primary key default gen_random_uuid(),
    org_id       uuid not null references orgs (id) on delete cascade,
    name         text not null,
    -- peppered sha-256 of the bearer token; the plaintext is shown once, at
    -- creation, and never stored
    token_hash   text not null unique,
    created_by   uuid references users (id) on delete set null,
    created_at   timestamptz not null default now(),
    last_used_at timestamptz,
    revoked_at   timestamptz
);

create index if not exists scim_tokens_org_idx on scim_tokens (org_id);

-- the SCIM view of a local user, scoped to the org that provisioned them. kept
-- beside `users` rather than on it: a user may exist without SCIM, and the IdP
-- owns userName/externalId only within its own org.
create table if not exists scim_identities (
    user_id      uuid not null references users (id) on delete cascade,
    org_id       uuid not null references orgs (id) on delete cascade,
    -- the IdP's stable id for the resource; absent for IdPs that omit it
    external_id  text,
    -- SCIM userName, unique within the org
    user_name    text not null,
    display_name text not null default '',
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now(),
    primary key (user_id, org_id),
    constraint scim_identities_org_user_name_unique unique (org_id, user_name)
);

create unique index if not exists scim_identities_org_external_id_unique
    on scim_identities (org_id, external_id)
    where external_id is not null;
