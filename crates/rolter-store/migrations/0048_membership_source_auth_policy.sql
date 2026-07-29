-- Make local invitations and SSO co-exist (#240).
--
-- Two independent problems:
--
-- 1. A membership needs to remember who granted it. Each SSO login recomputes
--    the roles the IdP's groups imply; without a source marker that
--    reconciliation would also delete a role an admin granted by hand (or
--    through an invitation), silently undoing operator intent.
-- 2. A deployment must be able to say "this org logs in through the IdP only"
--    without that also locking out the break-glass superadmin.

alter table memberships
    add column if not exists source text not null default 'manual';

do $$
begin
    alter table memberships
        add constraint memberships_source_check check (source in ('manual', 'sso'));
exception
    when duplicate_object then null;
end
$$;

create index if not exists memberships_user_source_idx on memberships (user_id, source);

-- per-org login policy. absent row means "both methods allowed", which is what
-- every existing org gets: adding sso must not change how anyone logs in today.
create table if not exists org_auth_policies (
    org_id               uuid primary key references orgs (id) on delete cascade,
    -- when false, email+password login is refused for members of this org.
    -- superadmins are exempt: a broken IdP would otherwise lock the deployment
    -- out permanently, with no path back in.
    allow_password_login boolean not null default true,
    -- when false, sso callbacks for this org's providers are refused without
    -- deleting the provider rows, so an operator can disable an IdP fast
    allow_sso            boolean not null default true,
    updated_at           timestamptz not null default now()
);
