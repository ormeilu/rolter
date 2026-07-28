-- prompt-template edits affect snapshot prompt_templates, so they must bump
-- config_version atomically with the same transaction

drop trigger if exists prompt_templates_bump_config_version on prompt_templates;
create trigger prompt_templates_bump_config_version
    after insert or update or delete on prompt_templates
    for each statement execute function bump_config_version();

drop trigger if exists prompt_template_versions_bump_config_version on prompt_template_versions;
create trigger prompt_template_versions_bump_config_version
    after insert or update or delete on prompt_template_versions
    for each statement execute function bump_config_version();

drop trigger if exists prompt_template_scopes_bump_config_version on prompt_template_scopes;
create trigger prompt_template_scopes_bump_config_version
    after insert or update or delete on prompt_template_scopes
    for each statement execute function bump_config_version();
