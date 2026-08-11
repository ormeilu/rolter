-- allow routes and provider groups to select the lora_aware strategy (#853).
-- adapter residency is learned from traffic, so a route that switches to it
-- starts with an empty slot set and behaves like the pipeline stack until the
-- first requests populate it; widening the constraint moves no traffic on its
-- own.
alter table routes drop constraint if exists routes_strategy_check;
alter table routes
    add constraint routes_strategy_check
    check (strategy in (
        'round_robin', 'random', 'power_of_two', 'consistent_hash',
        'cache_aware', 'weighted', 'pipeline', 'cheapest', 'fastest',
        'precise_cache_aware', 'lmcache_aware', 'adaptive', 'lora_aware'
    ));

alter table provider_groups drop constraint if exists provider_groups_strategy_check;
alter table provider_groups
    add constraint provider_groups_strategy_check
    check (strategy in (
        'round_robin', 'random', 'power_of_two', 'consistent_hash',
        'cache_aware', 'weighted', 'pipeline', 'cheapest', 'fastest',
        'precise_cache_aware', 'lmcache_aware', 'adaptive', 'lora_aware'
    ));
