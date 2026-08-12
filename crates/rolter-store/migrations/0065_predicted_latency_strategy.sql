-- allow routes and provider groups to select the predicted_latency strategy (#853).
-- the per-target latency models are learned from completed requests, so a route
-- that switches to it predicts nothing until each target has served enough
-- traffic and behaves like the least-load pipeline in the meantime; widening the
-- constraint moves no traffic on its own.
alter table routes drop constraint if exists routes_strategy_check;
alter table routes
    add constraint routes_strategy_check
    check (strategy in (
        'round_robin', 'random', 'power_of_two', 'consistent_hash',
        'cache_aware', 'weighted', 'pipeline', 'cheapest', 'fastest',
        'precise_cache_aware', 'lmcache_aware', 'adaptive', 'lora_aware',
        'predicted_latency'
    ));

alter table provider_groups drop constraint if exists provider_groups_strategy_check;
alter table provider_groups
    add constraint provider_groups_strategy_check
    check (strategy in (
        'round_robin', 'random', 'power_of_two', 'consistent_hash',
        'cache_aware', 'weighted', 'pipeline', 'cheapest', 'fastest',
        'precise_cache_aware', 'lmcache_aware', 'adaptive', 'lora_aware',
        'predicted_latency'
    ));
