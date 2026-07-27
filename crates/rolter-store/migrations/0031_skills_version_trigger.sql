-- skills repository updates affect downstream runtime consumers, so they
-- should bump config_version atomically with the write transaction

drop trigger if exists skills_bump_config_version on skills;
create trigger skills_bump_config_version
    after insert or update or delete on skills
    for each statement execute function bump_config_version();

drop trigger if exists skill_versions_bump_config_version on skill_versions;
create trigger skill_versions_bump_config_version
    after insert or update or delete on skill_versions
    for each statement execute function bump_config_version();
