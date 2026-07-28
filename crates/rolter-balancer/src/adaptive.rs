//! Adaptive routing (#544): a weighted blend of observed latency, catalog cost
//! and in-flight load that only takes over once it can justify itself.
//!
//! The strategy is deliberately conservative. Three independent conditions must
//! all hold before a single request is routed by the blend:
//!
//! 1. the operator turned it on ([`AdaptiveRoutingConfig::enabled`] — the kill
//!    switch),
//! 2. the blend has some non-zero weight to rank with, and
//! 3. the route has served `min_samples` requests *and* the dominant signal has
//!    real evidence (at least two targets with a latency sample).
//!
//! Otherwise every pick goes to the same `pipeline` stack the route would have
//! used without the strategy, so switching a route to `adaptive` never moves
//! traffic on its own. When the blend is engaged, a bounded `exploration_ratio`
//! share of picks is made uniformly at random so a target the blend has learned
//! to avoid keeps producing fresh latency samples instead of going dark.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

use rand::RngExt;
use rolter_core::AdaptiveRoutingConfig;

use crate::scorer::{
    CheapestScorer, FastestScorer, LatencySource, LeastLoadScorer, Pipeline, Scorer,
};
use crate::{LoadBalancer, RouteContext};

/// Latency samples needed before the blend is trusted to rank targets: ranking
/// is relative, so a single sampled target says nothing about the others.
const MIN_LATENCY_SAMPLES: usize = 2;

pub struct Adaptive {
    cfg: AdaptiveRoutingConfig,
    /// route width
    n: usize,
    /// the weighted latency/cost/load blend
    blend: Pipeline,
    /// the deterministic stack served while the blend is not engaged
    fallback: Pipeline,
    /// live latency handle, also used as the evidence check
    latency: Option<Arc<dyn LatencySource>>,
    /// requests this route has observed since the balancer was built
    served: AtomicU64,
}

impl Adaptive {
    /// Build over a route's `weights`, per-target `costs` (`<= 0` = unknown,
    /// see [`CheapestScorer`]) and optional live latency handle.
    pub fn new(
        weights: &[u32],
        costs: &[f64],
        latency: Option<Arc<dyn LatencySource>>,
        cfg: &AdaptiveRoutingConfig,
    ) -> Self {
        let cfg = cfg.sanitized();
        let n = weights.len();
        let mut blend = Pipeline::new(n).named("adaptive");
        if let (Some(source), true) = (latency.as_ref(), cfg.latency_weight > 0.0) {
            blend = blend.with(
                Box::new(FastestScorer::new(n, source.clone())) as Box<dyn Scorer>,
                cfg.latency_weight,
            );
        }
        if cfg.cost_weight > 0.0 {
            let mut costs = costs.to_vec();
            costs.resize(n, 0.0);
            blend = blend.with(Box::new(CheapestScorer::new(&costs)), cfg.cost_weight);
        }
        if cfg.load_weight > 0.0 {
            blend = blend.with(Box::new(LeastLoadScorer::new(n)), cfg.load_weight);
        }
        Self {
            cfg,
            n,
            blend,
            fallback: Pipeline::default_stack(weights),
            latency,
            served: AtomicU64::new(0),
        }
    }

    /// Whether the blend may route this request.
    fn engaged(&self) -> bool {
        self.cfg.enabled
            && self.cfg.has_signal()
            && self.served.load(Relaxed) >= u64::from(self.cfg.min_samples)
            && self.evidence_ready()
    }

    /// Whether the dominant signal can actually separate two targets. Cost is a
    /// build-time constant and load is always readable, so only the latency
    /// signal — the one that needs traffic to exist — is checked here.
    fn evidence_ready(&self) -> bool {
        if self.cfg.latency_weight <= 0.0 {
            return true;
        }
        let Some(source) = &self.latency else {
            return false;
        };
        source
            .latencies(self.n)
            .iter()
            .filter(|v| **v > 0.0)
            .count()
            >= MIN_LATENCY_SAMPLES
    }

    /// Whether this pick is spent on exploration rather than exploitation.
    fn explore(&self) -> bool {
        self.cfg.exploration_ratio > 0.0
            && self.n > 1
            && rand::rng().random::<f32>() < self.cfg.exploration_ratio
    }
}

impl LoadBalancer for Adaptive {
    fn name(&self) -> &'static str {
        "adaptive"
    }

    fn pick(&self, ctx: &RouteContext, loads: &[u64]) -> Option<usize> {
        if !self.engaged() {
            return self.fallback.pick(ctx, loads);
        }
        if self.explore() {
            return Some(rand::rng().random_range(0..self.n));
        }
        self.blend.pick(ctx, loads)
    }

    fn observe(&self, target: usize, ctx: &RouteContext) {
        self.served.fetch_add(1, Relaxed);
        // the fallback stack keeps learning while the blend is engaged, so a
        // config change that disengages adaptive routing lands on a warm
        // session/prefix cache rather than a cold one
        self.fallback.observe(target, ctx);
        self.blend.observe(target, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedLatency(Vec<f64>);

    impl LatencySource for FixedLatency {
        fn latencies(&self, n: usize) -> Vec<f64> {
            let mut out = self.0.clone();
            out.resize(n, 0.0);
            out
        }
    }

    fn cfg(enabled: bool, min_samples: u32) -> AdaptiveRoutingConfig {
        AdaptiveRoutingConfig {
            enabled,
            exploration_ratio: 0.0,
            min_samples,
            ..Default::default()
        }
    }

    fn warm(lb: &Adaptive, picks: u32) {
        for _ in 0..picks {
            lb.observe(0, &RouteContext::default());
        }
    }

    /// target 1 is both faster and cheaper, so an engaged blend always picks it
    fn lopsided(cfg: &AdaptiveRoutingConfig) -> Adaptive {
        Adaptive::new(
            &[1, 1],
            &[10.0, 1.0],
            Some(Arc::new(FixedLatency(vec![500.0, 10.0]))),
            cfg,
        )
    }

    #[test]
    fn kill_switch_keeps_traffic_on_the_fallback_stack() {
        let lb = lopsided(&cfg(false, 0));
        warm(&lb, 100);
        // the fallback stack is weight+load+prefix based and both targets are
        // identical there, so the disabled blend cannot pin traffic to target 1
        let mut picks = [0usize; 2];
        for _ in 0..200 {
            picks[lb.pick(&RouteContext::default(), &[]).unwrap()] += 1;
        }
        assert!(
            picks[0] > 0,
            "disabled adaptive routing still shifted all traffic: {picks:?}"
        );
    }

    #[test]
    fn engages_only_after_min_samples() {
        let lb = lopsided(&cfg(true, 10));
        assert!(!lb.engaged());
        warm(&lb, 9);
        assert!(!lb.engaged());
        warm(&lb, 1);
        assert!(lb.engaged());
        for _ in 0..50 {
            assert_eq!(lb.pick(&RouteContext::default(), &[]), Some(1));
        }
    }

    #[test]
    fn thin_latency_evidence_falls_back_deterministically() {
        // only one target has ever been sampled: nothing to rank against
        let lb = Adaptive::new(
            &[1, 1],
            &[10.0, 1.0],
            Some(Arc::new(FixedLatency(vec![500.0, 0.0]))),
            &cfg(true, 0),
        );
        assert!(!lb.engaged());
        // and with no latency handle at all the blend never engages either
        let blind = Adaptive::new(&[1, 1], &[10.0, 1.0], None, &cfg(true, 0));
        assert!(!blind.engaged());
    }

    #[test]
    fn cost_only_blend_needs_no_latency_evidence() {
        let policy = AdaptiveRoutingConfig {
            enabled: true,
            latency_weight: 0.0,
            load_weight: 0.0,
            exploration_ratio: 0.0,
            min_samples: 0,
            ..Default::default()
        };
        let lb = Adaptive::new(&[1, 1], &[10.0, 1.0], None, &policy);
        assert!(lb.engaged());
        assert_eq!(lb.pick(&RouteContext::default(), &[]), Some(1));
    }

    #[test]
    fn all_zero_weights_are_not_a_random_balancer() {
        let policy = AdaptiveRoutingConfig {
            enabled: true,
            latency_weight: 0.0,
            cost_weight: 0.0,
            load_weight: 0.0,
            min_samples: 0,
            ..Default::default()
        };
        let lb = lopsided(&policy);
        assert!(!lb.engaged());
    }

    #[test]
    fn exploration_ratio_is_bounded() {
        let policy = AdaptiveRoutingConfig {
            enabled: true,
            exploration_ratio: 9.0,
            min_samples: 0,
            ..Default::default()
        };
        let lb = lopsided(&policy);
        assert_eq!(lb.cfg.exploration_ratio, rolter_core::MAX_EXPLORATION_RATIO);
        // an engaged blend with capped exploration still favours the good
        // target by a wide margin
        let mut best = 0;
        for _ in 0..400 {
            if lb.pick(&RouteContext::default(), &[]) == Some(1) {
                best += 1;
            }
        }
        assert!(best > 250, "exploration swamped exploitation: {best}/400");
    }

    #[test]
    fn negative_weights_are_clamped() {
        let policy = AdaptiveRoutingConfig {
            enabled: true,
            cost_weight: -3.0,
            exploration_ratio: -1.0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(policy.cost_weight, 0.0);
        assert_eq!(policy.exploration_ratio, 0.0);
    }
}
