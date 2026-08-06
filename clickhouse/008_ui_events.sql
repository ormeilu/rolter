-- rolter dashboard UX events (clickhouse) — #805
--
-- Browser spans give latency and errors. They do not say whether a screen is
-- *usable*: which screens are slow to become interactive, where people back
-- out, which forms get abandoned, which error states are actually hit. This is
-- that stream, stored next to request_logs rather than in a second analytics
-- vendor — same deployment, same retention, same redaction, and it joins
-- directly against gateway traffic on trace_id.
--
-- Events are **structural only**. Every column is a key, an enum, a duration or
-- an id: there is deliberately no column that could hold a form value, prompt
-- text or free-text field contents, so the schema itself is the guarantee
-- rather than a policy applied on write.
--
-- Same plumbing as request_logs: append-only, batched off the hot path,
-- partitioned by day, 90-day TTL.

create table if not exists ui_events (
    ts          DateTime64(3) default now64(3),
    event_id    String,
    -- links a UX event to the gateway request it caused; empty when the
    -- interaction issued no request, or when browser tracing is off
    trace_id    String,
    session_id  String,
    org_id      String,
    team_id     String,
    project_id  String,
    user_id     String,
    -- stable screen key (route id), not a URL: a URL can carry ids and filters
    screen      LowCardinality(String),
    action      Enum8(
        'screen_view'        = 1,
        'time_to_interactive'= 2,
        'navigate'           = 3,
        'back_out'           = 4,
        'form_submit'        = 5,
        'form_abandon'       = 6,
        'validation_error'   = 7,
        'empty_state'        = 8,
        'error_state'        = 9,
        'save_confirmed'     = 10
    ),
    -- which form, control or validation rule — the *name*, never the value
    target      LowCardinality(String),
    outcome     Enum8('ok' = 1, 'error' = 2, 'cancelled' = 3),
    -- time-to-interactive, form dwell, or save-to-confirmation, per action
    duration_ms UInt32,
    -- previous screen, so navigation paths and back-outs are queryable
    from_screen LowCardinality(String),
    app_version LowCardinality(String)
)
engine = MergeTree
partition by toYYYYMMDD(ts)
order by (ts, screen, action)
ttl toDateTime(ts) + interval 90 day;
