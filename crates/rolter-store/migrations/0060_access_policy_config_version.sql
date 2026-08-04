-- Propagate access-profile model/route policy to the data plane (#791).
--
-- 0058 stated that none of the RBAC tables carries a `bump_config_version()`
-- trigger and that none may grow one, because the data plane did not consume
-- them. That was true when it was written and is the reason enforcement was
-- deferred; this migration changes the premise rather than breaking the rule.
-- The gateway now enforces the merged model/route policy carried by the access
-- profiles a virtual key's owner holds, so these four tables *are* data-plane
-- input and must bump the version like every other table the snapshot reads.
--
-- Only the tables that feed that resolution are covered. `custom_roles` and
-- `custom_role_grants` decide control-plane authorization, which is still
-- evaluated per request against the live database, so they stay untriggered:
-- waking the gateway fleet for a grant edit it cannot observe is exactly the
-- cost 0058 was avoiding.

-- the policy itself: the allow/deny lists the gateway matches against
drop trigger if exists access_profile_policies_bump_config_version
    on access_profile_policies;
create trigger access_profile_policies_bump_config_version
    after insert or update or delete on access_profile_policies
    for each statement execute function bump_config_version();

-- who holds the profile. Assigning or revoking a profile changes which policy
-- applies to a key without touching the policy row itself
drop trigger if exists access_profile_assignments_bump_config_version
    on access_profile_assignments;
create trigger access_profile_assignments_bump_config_version
    after insert or update or delete on access_profile_assignments
    for each statement execute function bump_config_version();

-- deleting a profile cascades to both tables above. Postgres fires statement
-- triggers on the cascaded deletes too, so this is belt-and-braces rather than
-- load-bearing — but a profile row that exists with neither an assignment nor a
-- policy is still the anchor both cascade from, and a future column here would
-- otherwise propagate silently
drop trigger if exists access_profiles_bump_config_version on access_profiles;
create trigger access_profiles_bump_config_version
    after insert or update or delete on access_profiles
    for each statement execute function bump_config_version();

-- team membership is the second path a profile reaches a user by: a profile
-- assigned to a team applies to everyone in it, so adding or removing a member
-- changes that user's effective policy with no row in the tables above
-- changing at all. SCIM syncs can move many rows at once, but this is a
-- statement-level trigger and the gateway coalesces on the version number, so a
-- bulk sync costs one snapshot poll rather than one per member.
drop trigger if exists memberships_bump_config_version on memberships;
create trigger memberships_bump_config_version
    after insert or update or delete on memberships
    for each statement execute function bump_config_version();
