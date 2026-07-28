-- foundational governance entities for org-scoped cost attribution
create table if not exists business_units (
    id          uuid primary key default gen_random_uuid(),
    org_id      uuid not null references orgs(id) on delete cascade,
    name        text not null check (length(trim(name)) > 0),
    slug        text not null check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    retired_at  timestamptz,
    created_at  timestamptz not null default now(),
    unique (org_id, slug),
    unique (id, org_id)
);

create table if not exists customers (
    id                uuid primary key default gen_random_uuid(),
    org_id            uuid not null references orgs(id) on delete cascade,
    business_unit_id  uuid,
    name              text not null check (length(trim(name)) > 0),
    slug              text not null check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    retired_at        timestamptz,
    created_at        timestamptz not null default now(),
    unique (org_id, slug),
    foreign key (business_unit_id, org_id)
        references business_units(id, org_id) on delete restrict
);
