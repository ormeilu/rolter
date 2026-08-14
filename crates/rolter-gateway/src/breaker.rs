//! Per-target circuit breaker (closed / open / half-open). Complements the
//! short-lived [`crate::cooldowns`] park: a cooldown shrugs off a single wobble,
//! the breaker sheds sustained load off a target that is down hard. State lives
//! outside the routing snapshot (it must survive config hot-reloads) and is keyed
//! by `(public model, target index)`.
//!
//! State machine per target:
//! - **Closed**: traffic flows; consecutive transient failures are counted. When
//!   the count reaches `failure_threshold` the target trips **open**.
//! - **Open**: traffic is skipped until `open_secs` elapse, then the next probe is
//!   admitted, moving the target to **half-open**.
//! - **Half-open**: a single probe is allowed through; a success closes the
//!   breaker (reset), a failure re-opens it for another `open_secs` window.
//!
//! A derived `Default` registry is permanently inert (no backing store) and
//! admits every target. A registry built with [`Breaker::new`] can be enabled,
//! disabled and re-tuned live by [`Breaker::reconfigure`] on a config hot-reload;
//! while disabled it admits every target and records nothing, but keeps its
//! per-target state so re-enabling resumes where it left off.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The phase a single target's breaker is in.
#[derive(Clone, Copy)]
enum Phase {
    Closed,
    /// tripped; skip traffic until this instant, then probe
    Open(Instant),
    /// probing; a single request has been admitted after the open window
    HalfOpen,
}

/// Per-target breaker state: its phase plus the running count of consecutive
/// transient failures observed while closed.
struct Entry {
    phase: Phase,
    consecutive_failures: u32,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            phase: Phase::Closed,
            consecutive_failures: 0,
        }
    }
}

/// Breaker state for one model's targets, addressed by target index.
///
/// A `Vec` rather than a second map: target indices are positions in a route's
/// target list, so they are small and dense, and indexing costs a bounds check
/// instead of a second hash. `None` is a target with no recorded outcome, which
/// is indistinguishable from a closed one.
type ModelBreakers = Vec<Option<Entry>>;

/// Per-target breaker state keyed by public model, then addressed by target index.
///
/// Split rather than a flat `(String, usize)` key: a flat tuple cannot be looked
/// up from a `&str`, so every read had to materialise a `String` just to hash
/// it. `allows` is a read-only admission check run per request per candidate
/// target — including the common case where the target was never seen and the
/// answer is an immediate `true` — so that allocation sat on the hottest path in
/// the gateway. Splitting keeps reads borrowed, hashes the model exactly once
/// and confines allocation to a model's first recorded failure (#1050).
type BreakerMap = HashMap<String, ModelBreakers>;

/// Shared, interior-mutable breaker state. The map holds per-target phase (kept
/// across config hot-reloads); the atomics hold the enable flag and tuning, which
/// [`Breaker::reconfigure`] updates in place so a reload can toggle or re-tune the
/// breaker without discarding accumulated per-target state.
struct Shared {
    map: Mutex<BreakerMap>,
    enabled: AtomicBool,
    failure_threshold: AtomicU32,
    open_secs: AtomicU64,
}

/// Shared, cheaply-cloneable circuit-breaker registry. A `None` inner is a
/// permanently inert breaker (used by embedders/tests that never reload); a
/// `Some` inner can be enabled, disabled and re-tuned live via [`Breaker::reconfigure`].
/// While disabled it admits every target and records nothing.
#[derive(Clone, Default)]
pub struct Breaker {
    inner: Option<Arc<Shared>>,
}

impl Breaker {
    /// A reconfigurable registry, initially `enabled` or not. `failure_threshold`
    /// consecutive failures trip a target open; it stays open for `open_secs`
    /// before a half-open probe. Build one even when disabled so a later reload can
    /// enable it in place.
    pub fn new(enabled: bool, failure_threshold: u32, open_secs: u64) -> Self {
        Self {
            inner: Some(Arc::new(Shared {
                map: Mutex::new(HashMap::new()),
                enabled: AtomicBool::new(enabled),
                failure_threshold: AtomicU32::new(failure_threshold.max(1)),
                open_secs: AtomicU64::new(open_secs),
            })),
        }
    }

    /// Apply new tuning from a config hot-reload. Toggles the enable flag and
    /// updates the thresholds atomically; the per-target phase map is preserved, so
    /// a target that is currently open stays open across a tuning-only reload. A
    /// permanently-inert breaker (`inner: None`) ignores the call.
    pub fn reconfigure(&self, enabled: bool, failure_threshold: u32, open_secs: u64) {
        let Some(inner) = &self.inner else {
            return;
        };
        inner
            .failure_threshold
            .store(failure_threshold.max(1), Relaxed);
        inner.open_secs.store(open_secs, Relaxed);
        inner.enabled.store(enabled, Relaxed);
    }

    /// Whether this registry is currently enforcing (enabled with a backing store).
    fn active(&self) -> Option<&Arc<Shared>> {
        let inner = self.inner.as_ref()?;
        inner.enabled.load(Relaxed).then_some(inner)
    }

    /// Whether `(model, idx)` may currently receive traffic. Closed and half-open
    /// targets are admitted; an open target is skipped until its window elapses,
    /// at which point the call transitions it to half-open and admits the probe.
    /// A disabled registry always admits.
    pub fn allows(&self, model: &str, idx: usize) -> bool {
        let Some(inner) = self.active() else {
            return true;
        };
        let mut map = inner.map.lock();
        // borrowed throughout: the never-seen case must not allocate, and it is
        // the overwhelmingly common one
        let Some(entry) = map
            .get_mut(model)
            .and_then(|m| m.get_mut(idx))
            .and_then(Option::as_mut)
        else {
            return true; // never-seen target is closed by default
        };
        match entry.phase {
            Phase::Closed | Phase::HalfOpen => true,
            Phase::Open(until) => {
                if Instant::now() >= until {
                    entry.phase = Phase::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Record a successful upstream response for `(model, idx)`. Resets the failure
    /// count and closes the breaker. Returns `true` when this closed a breaker that
    /// was previously open or half-open (a recovery worth counting).
    pub fn on_success(&self, model: &str, idx: usize) -> bool {
        let Some(inner) = self.active() else {
            return false;
        };
        let mut map = inner.map.lock();
        let Some(entry) = map
            .get_mut(model)
            .and_then(|m| m.get_mut(idx))
            .and_then(Option::as_mut)
        else {
            // a target with no entry is already closed with no failures, which
            // is exactly what this call would have written — so the success of
            // a healthy target neither allocates nor grows the map
            return false;
        };
        let was_tripped = !matches!(entry.phase, Phase::Closed);
        entry.phase = Phase::Closed;
        entry.consecutive_failures = 0;
        was_tripped
    }

    /// Record a transient failure for `(model, idx)`. A failure while half-open
    /// re-opens immediately; a closed target opens once its consecutive failures
    /// reach the threshold. Returns `true` when this call tripped the target open
    /// (a closed→open or half-open→open transition worth counting).
    pub fn on_failure(&self, model: &str, idx: usize) -> bool {
        let Some(inner) = self.active() else {
            return false;
        };
        let failure_threshold = inner.failure_threshold.load(Relaxed).max(1);
        let open_secs = inner.open_secs.load(Relaxed);
        let open_until = Instant::now() + Duration::from_secs(open_secs);
        let mut map = inner.map.lock();
        // a target that has already failed once is the common case while a
        // fault persists, and it needs no allocation; only a model's first
        // failure owns its name
        if let Some(entry) = map
            .get_mut(model)
            .and_then(|m| m.get_mut(idx))
            .and_then(Option::as_mut)
        {
            return record_failure(entry, failure_threshold, open_until);
        }
        // reached only on a target's *first* failure, at most once per target
        // per process, so owning the model name here costs nothing measurable
        let targets = map.entry(model.to_string()).or_default();
        if targets.len() <= idx {
            targets.resize_with(idx + 1, || None);
        }
        let entry = targets[idx].get_or_insert_with(Entry::default);
        record_failure(entry, failure_threshold, open_until)
    }
}

/// Fold one transient failure into a target's entry. Returns `true` when the
/// call tripped the target open (a closed→open or half-open→open transition).
fn record_failure(entry: &mut Entry, failure_threshold: u32, open_until: Instant) -> bool {
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    match entry.phase {
        // a failed probe drops straight back to open
        Phase::HalfOpen => {
            entry.phase = Phase::Open(open_until);
            true
        }
        Phase::Closed if entry.consecutive_failures >= failure_threshold => {
            entry.phase = Phase::Open(open_until);
            true
        }
        // already open, or not yet at threshold
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `std::sync::Mutex` would poison here and every later `allows()` would
    /// panic — one transient panic anywhere under the lock would turn into a
    /// permanent, restart-only outage for all traffic (#1049). `parking_lot`
    /// has no poison state, so the registry survives and keeps admitting.
    #[test]
    fn a_panic_under_the_lock_does_not_brick_the_registry() {
        let b = Breaker::new(true, 3, 30);
        // put real state in the map so recovery is observable, not vacuous
        b.on_failure("m", 0);
        b.on_failure("m", 0);
        let inner = b
            .inner
            .clone()
            .expect("an enabled breaker has a backing store");

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = inner.map.lock();
            panic!("a transient panic while the breaker map is locked");
        }));
        assert!(panicked.is_err(), "the test's own panic must have fired");

        // the lock is usable again and the pre-panic state is intact: the third
        // consecutive failure still trips the target, exactly as before
        assert!(b.allows("m", 0));
        assert!(b.on_failure("m", 0));
        assert!(!b.allows("m", 0));
        // and an unrelated target is still admitted rather than panicking
        assert!(b.allows("other", 7));
    }

    #[test]
    fn default_registry_is_inert() {
        let b = Breaker::default();
        assert!(b.allows("m", 0));
        // failures never trip a disabled breaker
        for _ in 0..100 {
            assert!(!b.on_failure("m", 0));
        }
        assert!(b.allows("m", 0));
    }

    #[test]
    fn trips_open_after_threshold() {
        let b = Breaker::new(true, 3, 30);
        // below threshold: still closed, still admitted
        assert!(!b.on_failure("m", 0));
        assert!(!b.on_failure("m", 0));
        assert!(b.allows("m", 0));
        // the third consecutive failure trips it open
        assert!(b.on_failure("m", 0));
        assert!(!b.allows("m", 0));
        // a distinct target is unaffected
        assert!(b.allows("m", 1));
    }

    #[test]
    fn success_resets_failure_count() {
        let b = Breaker::new(true, 3, 30);
        b.on_failure("m", 0);
        b.on_failure("m", 0);
        // a success clears the count so the next two failures do not trip it
        assert!(!b.on_success("m", 0));
        assert!(!b.on_failure("m", 0));
        assert!(!b.on_failure("m", 0));
        assert!(b.allows("m", 0));
    }

    #[test]
    fn half_open_probe_closes_on_success() {
        let b = Breaker::new(true, 1, 0); // open window of 0s → immediately probeable
        assert!(b.on_failure("m", 0)); // trips open
                                       // window already elapsed: the next allow admits a half-open probe
        assert!(b.allows("m", 0));
        // a success on the probe closes the breaker (a recovery)
        assert!(b.on_success("m", 0));
        assert!(b.allows("m", 0));
    }

    #[test]
    fn half_open_probe_reopens_on_failure() {
        let b = Breaker::new(true, 1, 0);
        assert!(b.on_failure("m", 0)); // trips open
        assert!(b.allows("m", 0)); // admits half-open probe
                                   // the probe fails: straight back to open, counted as a trip
        assert!(b.on_failure("m", 0));
    }

    #[test]
    fn reconfigure_can_disable_and_re_enable_preserving_state() {
        let b = Breaker::new(true, 1, 30);
        assert!(b.on_failure("m", 0)); // trips open
        assert!(!b.allows("m", 0));

        // disabling makes it inert: every target admitted, failures ignored
        b.reconfigure(false, 1, 30);
        assert!(b.allows("m", 0));
        assert!(!b.on_failure("m", 0));

        // re-enabling resumes the preserved phase — the target is still open
        b.reconfigure(true, 1, 30);
        assert!(!b.allows("m", 0));
    }

    #[test]
    fn reconfigure_retunes_threshold_in_place() {
        let b = Breaker::new(true, 5, 30);
        // below the original threshold of 5
        for _ in 0..3 {
            assert!(!b.on_failure("m", 0));
        }
        // loosen? no — tighten to 4: the 4th consecutive failure now trips it
        b.reconfigure(true, 4, 30);
        assert!(b.on_failure("m", 0));
        assert!(!b.allows("m", 0));
    }

    /// Number of `(model, target)` entries currently held, across all models.
    fn entries(b: &Breaker) -> usize {
        b.inner
            .as_ref()
            .map(|i| i.map.lock().values().map(|m| m.len()).sum())
            .unwrap_or(0)
    }

    /// A healthy target's success is indistinguishable from having no entry at
    /// all, so it must not create one — this is what keeps the all-healthy
    /// steady state both allocation-free and map-growth-free.
    #[test]
    fn a_success_on_a_never_seen_target_records_nothing() {
        let b = Breaker::new(true, 3, 30);
        for _ in 0..1_000 {
            assert!(!b.on_success("m", 0));
        }
        assert_eq!(entries(&b), 0, "healthy traffic must not populate the map");
        // and the target is of course still admitted
        assert!(b.allows("m", 0));
    }

    /// A success must still close a target that actually was tripped — the
    /// no-entry short-circuit above must not swallow a real recovery.
    #[test]
    fn a_success_still_closes_a_tripped_target() {
        let b = Breaker::new(true, 3, 30);
        b.on_failure("m", 0);
        b.on_failure("m", 0);
        assert!(b.on_failure("m", 0), "the third failure trips it open");
        assert!(!b.allows("m", 0));
        assert!(b.on_success("m", 0), "the recovery must be reported");
        assert!(b.allows("m", 0));
        // the failure count was reset too, not merely the phase: two more
        // failures must not be enough to trip it again
        assert!(!b.on_failure("m", 0));
        assert!(!b.on_failure("m", 0));
        assert!(b.allows("m", 0));
        assert_eq!(entries(&b), 1);
    }

    /// The key is nested now; two models sharing a target index must not alias.
    #[test]
    fn models_with_the_same_target_index_do_not_alias() {
        let b = Breaker::new(true, 1, 30);
        assert!(b.on_failure("a", 0)); // trips a/0 open
        assert!(!b.allows("a", 0));
        // same index, different model: untouched and still admitted
        assert!(b.allows("b", 0));
        assert!(b.on_failure("b", 0));
        assert!(!b.allows("b", 0));
        // and a's state did not move when b tripped
        assert!(!b.allows("a", 0));
        assert_eq!(entries(&b), 2);
    }

    /// A model name containing the separator-ish characters a flat key might
    /// have used must still be a distinct model.
    #[test]
    fn model_names_are_compared_whole() {
        let b = Breaker::new(true, 1, 30);
        assert!(b.on_failure("m::v", 0));
        assert!(!b.allows("m::v", 0));
        assert!(b.allows("m", 0));
    }

    #[test]
    fn reconfigure_is_a_noop_on_inert_default() {
        let b = Breaker::default();
        b.reconfigure(true, 1, 1);
        // still inert: no backing store to enable
        assert!(b.allows("m", 0));
        assert!(!b.on_failure("m", 0));
    }
}
