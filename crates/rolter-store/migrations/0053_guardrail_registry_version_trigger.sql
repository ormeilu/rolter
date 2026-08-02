-- guardrail changes are consumed by the data plane and must invalidate the
-- control-plane snapshot version in the same transaction
create trigger guardrail_rules_bump_config_version
after insert or update or delete on guardrail_rules
for each statement execute function bump_config_version();

create trigger guardrail_providers_bump_config_version
after insert or update or delete on guardrail_providers
for each statement execute function bump_config_version();
