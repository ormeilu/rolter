-- singleton logging policy projected into gateway snapshots for hot-reload
create table if not exists logging_settings (
    id                               boolean primary key default true check (id),
    sample_rate                      double precision not null default 1.0 check (sample_rate >= 0 and sample_rate <= 1),
    payload_capture_enabled          boolean not null default false,
    payload_capture_max_bytes        integer not null default 32768 check (payload_capture_max_bytes between 0 and 1048576),
    payload_capture_redact_fields    text[] not null default array['api_key','authorization','password','secret','token'],
    payload_capture_models           text[] not null default '{}',
    payload_capture_virtual_key_ids  text[] not null default '{}',
    updated_at                       timestamptz not null default now()
);

insert into logging_settings (id) values (true) on conflict (id) do nothing;

drop trigger if exists logging_settings_bump_config_version on logging_settings;
create trigger logging_settings_bump_config_version
after insert or update or delete on logging_settings
for each statement execute function bump_config_version();
