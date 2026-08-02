-- deployment-wide built-in rules and external semantic guardrail providers
create table guardrail_rules (
    id              uuid primary key default gen_random_uuid(),
    name            text not null unique check (length(trim(name)) > 0),
    enabled         boolean not null default true,
    source_type     text not null check (source_type in ('builtin', 'pattern')),
    builtin         text check (builtin in ('email', 'phone', 'api_token', 'payment_card')),
    pattern         text,
    stage           text not null default 'pre_call' check (stage in ('pre_call', 'post_call')),
    action          text not null default 'annotate' check (action in ('annotate', 'block', 'redact')),
    replacement     text,
    include_system  boolean not null default false,
    position        integer not null default 0 check (position >= 0),
    created_at      timestamptz not null default now(),
    updated_at      timestamptz not null default now(),
    check (
        (source_type = 'builtin' and builtin is not null and pattern is null)
        or
        (source_type = 'pattern' and builtin is null and pattern is not null and length(pattern) > 0)
    )
);

create table guardrail_providers (
    id              uuid primary key default gen_random_uuid(),
    name            text not null unique check (length(trim(name)) > 0),
    enabled         boolean not null default false,
    url             text not null check (url ~ '^https?://'),
    stage           text not null default 'pre_call' check (stage in ('pre_call', 'post_call')),
    timeout_ms      integer not null default 2000 check (timeout_ms > 0),
    max_retries     integer not null default 0 check (max_retries >= 0),
    failure_mode    text not null default 'fail_open' check (failure_mode in ('fail_open', 'fail_closed')),
    max_body_bytes  integer not null default 65536 check (max_body_bytes > 0),
    auth_kind       text not null default 'none' check (auth_kind in ('none', 'bearer', 'shared_secret')),
    auth_env        text,
    created_at      timestamptz not null default now(),
    updated_at      timestamptz not null default now(),
    check (
        (auth_kind = 'none' and auth_env is null)
        or
        (auth_kind <> 'none' and auth_env is not null and length(trim(auth_env)) > 0)
    )
);

-- the gateway has one vendor-neutral webhook contract, so at most one
-- registered provider may be active in the effective snapshot
create unique index guardrail_providers_one_enabled
    on guardrail_providers (enabled)
    where enabled;
