//! Admission-check microbenchmarks for the circuit breaker and cooldown
//! registries (#1050).
//!
//! `Breaker::allows` and `Cooldowns::is_parked` are consulted on every upstream
//! attempt, per candidate target, before anything else happens. They used to
//! build a `(String, usize)` key to hash — an allocation on the hottest path in
//! the gateway, including on the common case where the target was never seen
//! and the answer is an immediate "admitted". These benchmarks measure that
//! path so the change is settled by numbers rather than by reading.
//!
//! Run with `cargo bench -p rolter-gateway`.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use rolter_gateway::breaker::Breaker;
use rolter_gateway::cooldowns::Cooldowns;
use std::hint::black_box;

/// A realistic public model name: long enough that a per-call `String` copy is
/// a real allocation rather than something an allocator can hide.
const MODEL: &str = "anthropic/claude-sonnet-4-5-20250929";

/// Targets per route in the fleet shape these benches model.
const TARGETS: usize = 8;

fn breaker_admission(c: &mut Criterion) {
    let mut group = c.benchmark_group("breaker/allows");

    // the overwhelmingly common case: every target healthy, nothing recorded.
    // this is the one that used to allocate before returning `true`
    group.bench_function("cold_all_healthy", |b| {
        let breaker = Breaker::new(true, 5, 30);
        b.iter(|| {
            for idx in 0..TARGETS {
                black_box(breaker.allows(black_box(MODEL), black_box(idx)));
            }
        });
    });

    // a fleet that has seen failures, so every lookup hits a live entry
    group.bench_function("warm_entries_present", |b| {
        let breaker = Breaker::new(true, 5, 30);
        for idx in 0..TARGETS {
            breaker.on_failure(MODEL, idx);
        }
        b.iter(|| {
            for idx in 0..TARGETS {
                black_box(breaker.allows(black_box(MODEL), black_box(idx)));
            }
        });
    });

    // one target tripped open: the skip decision itself
    group.bench_function("one_target_open", |b| {
        let breaker = Breaker::new(true, 1, 3600);
        breaker.on_failure(MODEL, 0);
        b.iter(|| {
            for idx in 0..TARGETS {
                black_box(breaker.allows(black_box(MODEL), black_box(idx)));
            }
        });
    });

    // a gateway fronting many routes: the model map is wide, so the model-level
    // lookup is doing real work rather than hitting a single-entry map
    group.bench_function("cold_wide_fleet", |b| {
        let breaker = Breaker::new(true, 5, 30);
        let models: Vec<String> = (0..64).map(|i| format!("{MODEL}-{i}")).collect();
        for m in &models {
            breaker.on_failure(m, 0);
            breaker.on_success(m, 0);
        }
        b.iter(|| {
            for m in &models {
                for idx in 0..TARGETS {
                    black_box(breaker.allows(black_box(m.as_str()), black_box(idx)));
                }
            }
        });
    });

    group.finish();
}

fn breaker_recording(c: &mut Criterion) {
    let mut group = c.benchmark_group("breaker/record");

    // a success on a healthy target: the per-request outcome path
    group.bench_function("success_healthy", |b| {
        let breaker = Breaker::new(true, 5, 30);
        b.iter(|| black_box(breaker.on_success(black_box(MODEL), black_box(0))));
    });

    // a failure against a target that already has an entry: the sustained-fault
    // path, which must not re-allocate the model name per call
    group.bench_function("failure_entry_present", |b| {
        let breaker = Breaker::new(true, u32::MAX, 30);
        breaker.on_failure(MODEL, 0);
        b.iter(|| black_box(breaker.on_failure(black_box(MODEL), black_box(0))));
    });

    group.finish();
}

fn cooldown_admission(c: &mut Criterion) {
    let mut group = c.benchmark_group("cooldowns/is_parked");

    // nothing parked: the healthy steady state, and the former allocation site
    group.bench_function("cold_all_healthy", |b| {
        let cooldowns = Cooldowns::new();
        b.iter(|| {
            for idx in 0..TARGETS {
                black_box(cooldowns.is_parked(black_box(MODEL), black_box(idx)));
            }
        });
    });

    // one target parked, the rest healthy: the mixed case a real fleet sits in
    group.bench_function("one_target_parked", |b| {
        let cooldowns = Cooldowns::new();
        cooldowns.park(MODEL, 0, 3600);
        b.iter(|| {
            for idx in 0..TARGETS {
                black_box(cooldowns.is_parked(black_box(MODEL), black_box(idx)));
            }
        });
    });

    group.finish();
}

fn cooldown_parking(c: &mut Criterion) {
    let mut group = c.benchmark_group("cooldowns/park");

    // re-parking an already-parked model: the sustained-fault path, which now
    // takes the borrowed branch
    group.bench_function("repark_known_model", |b| {
        let cooldowns = Cooldowns::new();
        cooldowns.park(MODEL, 0, 3600);
        b.iter(|| cooldowns.park(black_box(MODEL), black_box(0), black_box(30)));
    });

    // a model's first park, which legitimately owns its name
    group.bench_function("first_park_new_model", |b| {
        b.iter_batched(
            Cooldowns::new,
            |cooldowns| cooldowns.park(black_box(MODEL), black_box(0), black_box(30)),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    breaker_admission,
    breaker_recording,
    cooldown_admission,
    cooldown_parking
);
criterion_main!(benches);
