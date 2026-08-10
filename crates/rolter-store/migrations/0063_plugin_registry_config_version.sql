-- plugin_instances rows are now consumed by the gateway's dispatch runtime
-- (#509); 0054 deliberately left this trigger out since dispatch did not
-- exist yet, so writes must start invalidating the control-plane snapshot
create trigger plugin_instances_bump_config_version
after insert or update or delete on plugin_instances
for each statement execute function bump_config_version();
