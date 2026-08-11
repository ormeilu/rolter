//! Background scrape of upstream engine `/metrics`. When enabled, a task pulls
//! each provider's Prometheus text exposition periodically, parses out the
//! signals below, and publishes them into a lock-free [`arc_swap`] snapshot.
//!
//! The scrape feeds two consumers:
//!
//! - **routing** — the request path folds the per-provider queue depth into the
//!   balancer's in-flight load view so load-aware strategies steer away from
//!   backed-up engines;
//! - **observability** (#850) — [`UpstreamMetrics::render`] re-exports the whole
//!   set through rolter's own `/metrics`, labelled by provider, so one scrape
//!   point covers the gateway and the engines behind it and an operator can
//!   correlate engine saturation with the target rolter chose.
//!
//! State lives outside the routing snapshot so it survives config hot-reloads,
//! and reads never take a lock (honoring the "no locks on the hot path" rule).
//! A disabled registry (the derived default) reports zero depth for everything
//! and exports nothing, leaving the balancer's own in-flight counts untouched.
//! A slow or dead upstream cannot affect the request path: the sweep runs on its
//! own task under a timeout, and a failed scrape stores the zero-valued default
//! rather than blocking or evicting anything.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use rolter_core::{GatewayConfig, MetricsScrapeConfig};

/// Per-provider scraped signals.
///
/// `queue_depth` is the one the balancer consumes; the rest exist so the
/// scrape is also usable for observability (#850) — an operator scraping
/// rolter's own `/metrics` sees the engine's numbers next to the routing
/// decision rolter made, without a second scrape target per engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderMetrics {
    /// requests waiting to be scheduled upstream (higher = more backed up)
    pub queue_depth: u64,
    /// requests the engine is currently decoding
    pub running_requests: u64,
    /// fraction of the KV cache in use, 0.0–1.0. `None` when the engine does
    /// not expose it — a missing gauge and a genuine 0% are not the same
    /// saturation story, so the series is omitted rather than reported as 0
    pub kv_cache_utilization: Option<f64>,
    /// engine-reported generation throughput in tokens/second
    pub generation_throughput: Option<f64>,
    /// whether the most recent sweep reached this provider. Exported so a dead
    /// upstream is visible as such instead of looking like an idle one
    pub scrape_ok: bool,
}

/// Provider name to its latest scraped metrics.
type MetricsMap = HashMap<String, ProviderMetrics>;

/// Shared, cheaply-cloneable snapshot of upstream metrics. The derived default
/// has no backing store and reports zero depth for every provider — i.e.
/// scraping is inert and contributes nothing to the load view.
#[derive(Clone, Default)]
pub struct UpstreamMetrics {
    inner: Option<Arc<ArcSwap<MetricsMap>>>,
}

impl UpstreamMetrics {
    /// An enabled registry with an empty snapshot. Until the first scrape lands,
    /// every provider reports zero depth.
    pub fn new() -> Self {
        Self {
            inner: Some(Arc::new(ArcSwap::from_pointee(HashMap::new()))),
        }
    }

    /// Latest scraped queue depth for `provider`. Unknown providers and a
    /// disabled registry both report `0`.
    pub fn queue_depth(&self, provider: &str) -> u64 {
        let Some(inner) = &self.inner else {
            return 0;
        };
        inner
            .load()
            .get(provider)
            .map(|m| m.queue_depth)
            .unwrap_or(0)
    }

    /// Atomically replace the whole snapshot with a freshly scraped sweep.
    fn store(&self, map: MetricsMap) {
        if let Some(inner) = &self.inner {
            inner.store(Arc::new(map));
        }
    }

    /// Append the latest scraped upstream series to a Prometheus text
    /// exposition (#850).
    ///
    /// Cardinality is bounded by the provider count — one label set per
    /// provider per series, no per-request or per-model dimension. A disabled
    /// registry emits nothing at all, not even the HELP/TYPE preamble, so a
    /// deployment that never enabled scraping does not advertise empty series.
    pub fn render(&self, out: &mut String) {
        use std::fmt::Write as _;

        let Some(inner) = &self.inner else {
            return;
        };
        let snapshot = inner.load();
        if snapshot.is_empty() {
            return;
        }

        for (name, help, kind) in [
            (
                "rolter_upstream_queue_depth",
                "requests waiting to be scheduled on the upstream engine",
                "gauge",
            ),
            (
                "rolter_upstream_running_requests",
                "requests currently decoding on the upstream engine",
                "gauge",
            ),
            (
                "rolter_upstream_kv_cache_utilization",
                "fraction of the upstream engine KV cache in use (0-1)",
                "gauge",
            ),
            (
                "rolter_upstream_generation_throughput_tokens_per_second",
                "generation throughput reported by the upstream engine",
                "gauge",
            ),
            (
                "rolter_upstream_scrape_up",
                "1 when the last sweep reached this provider, 0 otherwise",
                "gauge",
            ),
        ] {
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} {kind}");
        }

        // one pass per provider rather than per metric: the map is small and
        // this keeps the sample lines for a provider adjacent, which is legal
        // exposition and easier to eyeball
        for (provider, metrics) in snapshot.iter() {
            let provider = escape_label(provider);
            let _ = writeln!(
                out,
                "rolter_upstream_queue_depth{{provider=\"{provider}\"}} {}",
                metrics.queue_depth
            );
            let _ = writeln!(
                out,
                "rolter_upstream_running_requests{{provider=\"{provider}\"}} {}",
                metrics.running_requests
            );
            if let Some(util) = metrics.kv_cache_utilization {
                let _ = writeln!(
                    out,
                    "rolter_upstream_kv_cache_utilization{{provider=\"{provider}\"}} {util}"
                );
            }
            if let Some(tps) = metrics.generation_throughput {
                let _ = writeln!(
                    out,
                    "rolter_upstream_generation_throughput_tokens_per_second{{provider=\"{provider}\"}} {tps}"
                );
            }
            let _ = writeln!(
                out,
                "rolter_upstream_scrape_up{{provider=\"{provider}\"}} {}",
                u8::from(metrics.scrape_ok)
            );
        }
    }
}

/// Escape a Prometheus label value: backslash, double-quote and newline, per
/// the exposition format spec. Provider names are operator-supplied, so this
/// cannot be skipped.
fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// Prometheus metric names that expose the scheduler queue depth, in priority
/// order (vLLM, then generic/SGLang/TGI fallbacks). The first one present in a
/// scrape wins.
const QUEUE_DEPTH_METRICS: &[&str] = &[
    "vllm:num_requests_waiting",
    "sglang:num_queue_reqs",
    "num_requests_waiting",
    "tgi_queue_size",
];

/// Metric names exposing the count of requests currently decoding.
const RUNNING_REQUESTS_METRICS: &[&str] = &[
    "vllm:num_requests_running",
    "sglang:num_running_reqs",
    "num_requests_running",
    "tgi_batch_current_size",
];

/// Metric names exposing KV-cache occupancy as a 0–1 fraction.
const KV_CACHE_METRICS: &[&str] = &[
    "vllm:gpu_cache_usage_perc",
    "sglang:token_usage",
    "gpu_cache_usage_perc",
];

/// Metric names exposing engine-reported generation throughput (tokens/sec).
const THROUGHPUT_METRICS: &[&str] = &[
    "vllm:avg_generation_throughput_toks_per_s",
    "sglang:gen_throughput",
    "avg_generation_throughput_toks_per_s",
];

/// Sum the samples of the first present metric from `names` in a Prometheus
/// text exposition. Comment lines (`# HELP`/`# TYPE`) are ignored; labelled
/// series (`name{...} value`) are summed across all label sets. Returns `None`
/// when none of the names appear, which lets a caller distinguish "engine does
/// not expose this" from "engine reports zero".
///
/// Borrows throughout and allocates nothing: a scrape body is large and this
/// runs once per provider per sweep.
fn parse_metric(body: &str, names: &[&str]) -> Option<f64> {
    for metric in names {
        let mut total = 0f64;
        let mut seen = false;
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // a sample line is `name value` or `name{labels} value`; match the
            // metric name up to a space or the opening brace
            let name_end = line.find([' ', '{']).unwrap_or(line.len());
            if &line[..name_end] != *metric {
                continue;
            }
            if let Some(val) = line.rsplit(char::is_whitespace).next() {
                if let Ok(v) = val.parse::<f64>() {
                    total += v;
                    seen = true;
                }
            }
        }
        if seen {
            return total.is_finite().then_some(total);
        }
    }
    None
}

/// Sum of the first present queue-depth metric, as a whole request count.
/// Negatives, NaN and a missing metric all read as `0` — the balancer's load
/// view has no way to represent "unknown", and treating it as "not backed up"
/// keeps a non-reporting engine eligible rather than silently draining it.
fn parse_queue_depth(body: &str) -> u64 {
    to_count(parse_metric(body, QUEUE_DEPTH_METRICS))
}

/// Clamp an optional gauge to a whole non-negative count.
fn to_count(value: Option<f64>) -> u64 {
    match value {
        Some(v) if v.is_finite() && v > 0.0 => v.round() as u64,
        _ => 0,
    }
}

/// Parse every signal rolter re-exports from one scrape body.
fn parse_provider_metrics(body: &str) -> ProviderMetrics {
    ProviderMetrics {
        queue_depth: parse_queue_depth(body),
        running_requests: to_count(parse_metric(body, RUNNING_REQUESTS_METRICS)),
        // engines disagree on the unit: vLLM reports a 0-1 fraction, some
        // exporters a 0-100 percentage. Normalize to a fraction so the series
        // means one thing, and drop non-finite values rather than export a NaN
        kv_cache_utilization: parse_metric(body, KV_CACHE_METRICS)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| if v > 1.0 { v / 100.0 } else { v }),
        generation_throughput: parse_metric(body, THROUGHPUT_METRICS)
            .filter(|v| v.is_finite() && *v >= 0.0),
        scrape_ok: true,
    }
}

/// Spawn the background scraper. Sweeps every provider in the current snapshot
/// once per `interval_secs`, issuing `GET {api_base}{path}`, parses the queue
/// depth, and publishes a fresh snapshot. Runs until the process exits. A no-op
/// (returns without spawning) when scraping is disabled.
pub fn spawn_scraper(config: &GatewayConfig, state: crate::state::AppState) {
    if !config.metrics_scrape.enabled {
        return;
    }
    let cfg = config.metrics_scrape.clone();
    tokio::spawn(async move {
        run_scraper(cfg, state).await;
    });
}

async fn run_scraper(cfg: MetricsScrapeConfig, state: crate::state::AppState) {
    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.interval_secs.max(1)));
    loop {
        ticker.tick().await;
        // read providers off the current snapshot each sweep so hot-reloads and
        // newly-added providers are picked up without restarting the scraper
        let providers: Vec<rolter_core::ProviderConfig> = {
            let snap = state.snapshot.load();
            snap.providers.values().cloned().collect()
        };
        let mut map = MetricsMap::with_capacity(providers.len());
        for provider in providers {
            let url = format!("{}{}", provider.api_base.trim_end_matches('/'), cfg.path);
            // a failed sweep records the zero-valued default with scrape_ok
            // false, so the balancer keeps treating the provider as unloaded
            // while `/metrics` still shows it as unreachable
            let metrics = match state.forwarder.client_for(&provider) {
                Ok(client) => match tokio::time::timeout(
                    Duration::from_secs(cfg.timeout_secs.max(1)),
                    client.get(&url).send(),
                )
                .await
                {
                    Ok(Ok(resp)) if resp.status().is_success() => match resp.text().await {
                        Ok(body) => parse_provider_metrics(&body),
                        Err(_) => ProviderMetrics::default(),
                    },
                    _ => ProviderMetrics::default(),
                },
                Err(_) => ProviderMetrics::default(),
            };
            map.insert(provider.name, metrics);
        }
        state.upstream_metrics.store(map);
        state
            .metrics
            .metrics_scrapes_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_reports_zero() {
        let m = UpstreamMetrics::default();
        assert_eq!(m.queue_depth("anything"), 0);
        // store is inert on a disabled registry
        m.store(HashMap::from([(
            "p".to_string(),
            ProviderMetrics {
                queue_depth: 9,
                ..Default::default()
            },
        )]));
        assert_eq!(m.queue_depth("p"), 0);
    }

    #[test]
    fn stores_and_reports_depth() {
        let m = UpstreamMetrics::new();
        // unknown provider reads zero
        assert_eq!(m.queue_depth("p"), 0);
        m.store(HashMap::from([(
            "p".to_string(),
            ProviderMetrics {
                queue_depth: 4,
                ..Default::default()
            },
        )]));
        assert_eq!(m.queue_depth("p"), 4);
    }

    #[test]
    fn parses_vllm_queue_depth() {
        let body = "\
# HELP vllm:num_requests_waiting Number of requests waiting to be processed.
# TYPE vllm:num_requests_waiting gauge
vllm:num_requests_waiting{model_name=\"llama\"} 7.0
";
        assert_eq!(parse_queue_depth(body), 7);
    }

    #[test]
    fn sums_labelled_series() {
        let body = "\
vllm:num_requests_waiting{engine=\"0\"} 3
vllm:num_requests_waiting{engine=\"1\"} 5
";
        assert_eq!(parse_queue_depth(body), 8);
    }

    #[test]
    fn falls_back_to_generic_metric() {
        let body = "num_requests_waiting 2\n";
        assert_eq!(parse_queue_depth(body), 2);
    }

    #[test]
    fn missing_metric_is_zero() {
        assert_eq!(parse_queue_depth("some_other_metric 5\n"), 0);
        assert_eq!(parse_queue_depth(""), 0);
    }

    /// a realistic vLLM scrape carrying every series rolter re-exports
    const VLLM_BODY: &str = "\
# HELP vllm:num_requests_waiting Number of requests waiting to be processed.
# TYPE vllm:num_requests_waiting gauge
vllm:num_requests_waiting{model_name=\"llama\"} 7.0
vllm:num_requests_running{model_name=\"llama\"} 3.0
vllm:gpu_cache_usage_perc{model_name=\"llama\"} 0.42
vllm:avg_generation_throughput_toks_per_s{model_name=\"llama\"} 128.5
";

    #[test]
    fn parses_every_exported_signal() {
        let m = parse_provider_metrics(VLLM_BODY);
        assert_eq!(m.queue_depth, 7);
        assert_eq!(m.running_requests, 3);
        assert_eq!(m.kv_cache_utilization, Some(0.42));
        assert_eq!(m.generation_throughput, Some(128.5));
        assert!(m.scrape_ok);
    }

    #[test]
    fn absent_gauges_stay_none_rather_than_zero() {
        // a missing gauge and a genuine 0% are different saturation stories
        let m = parse_provider_metrics("vllm:num_requests_waiting 1\n");
        assert_eq!(m.kv_cache_utilization, None);
        assert_eq!(m.generation_throughput, None);
        // counts have no "unknown", so they floor at 0
        assert_eq!(m.running_requests, 0);
    }

    #[test]
    fn kv_cache_percentage_is_normalized_to_a_fraction() {
        // some exporters report 0-100 rather than vLLM's 0-1
        let m = parse_provider_metrics("gpu_cache_usage_perc 87.5\n");
        assert_eq!(m.kv_cache_utilization, Some(0.875));
        // a value already in 0-1 is left alone
        let m = parse_provider_metrics("gpu_cache_usage_perc 0.5\n");
        assert_eq!(m.kv_cache_utilization, Some(0.5));
    }

    #[test]
    fn sglang_names_are_recognized() {
        let body = "sglang:num_queue_reqs 2\nsglang:num_running_reqs 5\nsglang:token_usage 0.3\n";
        let m = parse_provider_metrics(body);
        assert_eq!(m.queue_depth, 2);
        assert_eq!(m.running_requests, 5);
        assert_eq!(m.kv_cache_utilization, Some(0.3));
    }

    #[test]
    fn non_finite_and_negative_gauges_are_dropped() {
        let m = parse_provider_metrics(
            "gpu_cache_usage_perc -1\navg_generation_throughput_toks_per_s NaN\n",
        );
        assert_eq!(m.kv_cache_utilization, None);
        assert_eq!(m.generation_throughput, None);
    }

    #[test]
    fn render_emits_one_label_set_per_provider() {
        let m = UpstreamMetrics::new();
        m.store(HashMap::from([(
            "vllm-a".to_string(),
            parse_provider_metrics(VLLM_BODY),
        )]));
        let mut out = String::new();
        m.render(&mut out);

        assert!(out.contains("# TYPE rolter_upstream_queue_depth gauge"));
        assert!(out.contains("rolter_upstream_queue_depth{provider=\"vllm-a\"} 7"));
        assert!(out.contains("rolter_upstream_running_requests{provider=\"vllm-a\"} 3"));
        assert!(out.contains("rolter_upstream_kv_cache_utilization{provider=\"vllm-a\"} 0.42"));
        assert!(out.contains(
            "rolter_upstream_generation_throughput_tokens_per_second{provider=\"vllm-a\"} 128.5"
        ));
        assert!(out.contains("rolter_upstream_scrape_up{provider=\"vllm-a\"} 1"));
    }

    #[test]
    fn render_marks_an_unreachable_provider_down() {
        let m = UpstreamMetrics::new();
        // what the scrape loop stores when a sweep fails
        m.store(HashMap::from([(
            "dead".to_string(),
            ProviderMetrics::default(),
        )]));
        let mut out = String::new();
        m.render(&mut out);
        assert!(out.contains("rolter_upstream_scrape_up{provider=\"dead\"} 0"));
        assert!(out.contains("rolter_upstream_queue_depth{provider=\"dead\"} 0"));
        // absent gauges are omitted entirely rather than exported as zero
        assert!(!out.contains("rolter_upstream_kv_cache_utilization{provider=\"dead\"}"));
    }

    #[test]
    fn render_is_silent_when_scraping_is_disabled() {
        let mut out = String::new();
        UpstreamMetrics::default().render(&mut out);
        assert!(out.is_empty(), "disabled registry must advertise nothing");

        // enabled but pre-first-sweep is also silent rather than empty series
        let mut out = String::new();
        UpstreamMetrics::new().render(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn render_escapes_provider_names() {
        let m = UpstreamMetrics::new();
        m.store(HashMap::from([(
            "we\"ird\\name".to_string(),
            ProviderMetrics::default(),
        )]));
        let mut out = String::new();
        m.render(&mut out);
        assert!(out.contains(r#"provider="we\"ird\\name""#));
    }

    #[test]
    fn first_present_metric_wins_over_later() {
        // vllm metric present -> the later generic fallback is not summed in
        let body = "\
vllm:num_requests_waiting 1
num_requests_waiting 100
";
        assert_eq!(parse_queue_depth(body), 1);
    }
}
