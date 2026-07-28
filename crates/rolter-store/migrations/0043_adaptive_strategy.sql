-- allow routes and provider groups to select the adaptive strategy (#544).
-- the strategy is inert until the deployment-wide adaptive policy is enabled,
-- so widening the constraint moves no traffic on its own.
alter table routes drop constraint if exists routes_strategy_check;
alter table routes
    add constraint routes_strategy_check
    check (strategy in (
        'round_robin', 'random', 'power_of_two', 'consistent_hash',
        'cache_aware', 'weighted', 'pipeline', 'cheapest', 'fastest',
        'precise_cache_aware', 'lmcache_aware', 'adaptive'
    ));

alter table provider_groups drop constraint if exists provider_groups_strategy_check;
alter table provider_groups
    add constraint provider_groups_strategy_check
    check (strategy in (
        'round_robin', 'random', 'power_of_two', 'consistent_hash',
        'cache_aware', 'weighted', 'pipeline', 'cheapest', 'fastest',
        'precise_cache_aware', 'lmcache_aware', 'adaptive'
    ));
