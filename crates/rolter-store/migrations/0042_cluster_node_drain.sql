-- operator-requested node state; the node learns it on its next snapshot poll
-- and stops advertising readiness while draining (#543)
alter table cluster_nodes
    add column if not exists desired_state text not null default 'active'
        check (desired_state in ('active', 'draining')),
    add column if not exists state_changed_at timestamptz not null default now();
