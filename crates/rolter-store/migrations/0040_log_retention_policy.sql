-- admin-managed log retention: metadata rows and raw payloads expire on
-- separate clocks so prompt bodies can be dropped long before aggregates (#537)
alter table logging_settings
    add column if not exists retention_days integer not null default 90
        check (retention_days between 1 and 3650),
    add column if not exists payload_retention_hours integer not null default 168
        check (payload_retention_hours between 1 and 8760);
