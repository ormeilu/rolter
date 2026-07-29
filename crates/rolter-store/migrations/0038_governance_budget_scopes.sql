-- budgets and rate limits may cap a governance dimension, not just tenancy,
-- so a business unit or customer can carry its own spend/throughput cap (#539)
alter table budgets
    drop constraint if exists budgets_scope_type_check;

alter table budgets
    add constraint budgets_scope_type_check
    check (scope_type in ('org', 'team', 'project', 'virtual_key', 'business_unit', 'customer'));

alter table rate_limits
    drop constraint if exists rate_limits_scope_type_check;

alter table rate_limits
    add constraint rate_limits_scope_type_check
    check (scope_type in ('org', 'team', 'project', 'virtual_key', 'business_unit', 'customer'));
