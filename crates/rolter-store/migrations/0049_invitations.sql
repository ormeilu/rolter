-- Invitation-based onboarding (#712).
--
-- The point of an invitation is that the invitee chooses their own password,
-- so nobody else ever knows it. Only the peppered digest of the one-time token
-- is stored, exactly like sessions and virtual keys: a database dump alone must
-- not yield a usable invite link.

create table if not exists invitations (
    id           uuid primary key default gen_random_uuid(),
    org_id       uuid not null references orgs (id) on delete cascade,
    email        text not null,
    -- role and scope the acceptance grants, using the same "most specific
    -- non-null id" convention as memberships
    role         text not null check (role in ('admin', 'member', 'viewer')),
    team_id      uuid references teams (id) on delete cascade,
    project_id   uuid references projects (id) on delete cascade,
    -- peppered sha-256 of the token handed to the invitee; the token itself is
    -- returned once and never stored
    token_hash   text not null unique,
    invited_by   uuid references users (id) on delete set null,
    expires_at   timestamptz not null,
    accepted_at  timestamptz,
    revoked_at   timestamptz,
    created_at   timestamptz not null default now()
);

-- one live invitation per email per org: re-inviting should replace, not
-- accumulate, and a partial index keeps spent invitations out of the way
create unique index if not exists invitations_live_email_idx
    on invitations (org_id, lower(email))
    where accepted_at is null and revoked_at is null;

create index if not exists invitations_org_idx on invitations (org_id, created_at desc);
