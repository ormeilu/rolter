-- Singleton client and model settings for the Settings group (#564).
--
-- `client_settings` owns what the gateway does with client-supplied and
-- gateway-supplied headers on the upstream leg, plus the base URL the dashboard
-- advertises in its connection snippets. `model_defaults` owns the inference
-- parameters filled in when a client omits them. Both project into gateway
-- snapshots, so both bump config_version.

create table if not exists client_settings (
    id                 boolean primary key default true check (id),
    -- advertised to clients in the dashboard's connection snippets; null falls
    -- back to the origin the dashboard itself is served from
    public_base_url    text,
    -- inbound client headers forwarded verbatim to the upstream provider
    forwarded_headers  text[] not null default '{}',
    -- static headers the gateway adds to every upstream request
    injected_headers   jsonb not null default '{}'::jsonb,
    -- header carrying the client-supplied correlation id
    request_id_header  text not null default 'x-request-id'
        check (request_id_header ~ '^[A-Za-z0-9!#$%&''*+.^_`|~-]+$'),
    updated_at         timestamptz not null default now()
);

insert into client_settings (id) values (true) on conflict (id) do nothing;

drop trigger if exists client_settings_bump_config_version on client_settings;
create trigger client_settings_bump_config_version
after insert or update or delete on client_settings
for each statement execute function bump_config_version();

create table if not exists model_defaults (
    id                  boolean primary key default true check (id),
    -- master switch: off means the gateway forwards request bodies untouched,
    -- which is the behaviour every deployment had before this table existed
    enabled             boolean not null default false,
    default_model       text,
    default_temperature double precision check (default_temperature between 0 and 2),
    default_top_p       double precision check (default_top_p between 0 and 1),
    default_max_tokens  integer check (default_max_tokens between 1 and 1000000),
    updated_at          timestamptz not null default now()
);

insert into model_defaults (id) values (true) on conflict (id) do nothing;

drop trigger if exists model_defaults_bump_config_version on model_defaults;
create trigger model_defaults_bump_config_version
after insert or update or delete on model_defaults
for each statement execute function bump_config_version();
