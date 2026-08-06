use std::sync::atomic::{AtomicBool, Ordering};

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Set once an OTLP pipeline is installed by [`init`].
///
/// Every propagation entry point checks this first so a deployment with no
/// `OTEL_*` environment never builds a carrier, never touches the global
/// propagator and never allocates — the hot path stays exactly as it was before
/// context propagation existed.
static PIPELINE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether a real OTLP pipeline is installed.
///
/// Call this before doing any work to build a [`TraceCarrier`]: with no OTLP
/// endpoint configured there is no context to extract or inject, and the answer
/// is a single relaxed atomic load.
#[inline]
#[must_use]
pub fn is_active() -> bool {
    PIPELINE_ACTIVE.load(Ordering::Relaxed)
}

/// Inbound trace-context headers, normalized for the W3C propagator.
///
/// Built by the caller only when [`is_active`] is true. B3 headers are folded
/// into an equivalent `traceparent` on [`TraceCarrier::finish`], so a single
/// W3C propagator serves both wire formats and no extra propagator crate is
/// needed.
#[derive(Debug, Default, Clone)]
pub struct TraceCarrier {
    entries: Vec<(String, String)>,
}

impl TraceCarrier {
    /// Empty carrier.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one inbound header. Keys are lowercased; blank values are dropped
    /// so a header present but empty cannot shadow a usable one.
    pub fn insert(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        self.entries
            .push((key.to_ascii_lowercase(), value.to_string()));
    }

    /// Normalize B3 into `traceparent` when the caller sent only B3, and return
    /// the finished carrier.
    #[must_use]
    pub fn finish(mut self) -> Self {
        if self.get("traceparent").is_none() {
            if let Some(tp) = self.b3_as_traceparent() {
                self.entries.push(("traceparent".to_string(), tp));
            }
        }
        self
    }

    /// True when the caller propagated no usable trace context.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Build a `traceparent` out of B3 single or multi headers, when both a
    /// well-formed trace id and span id are present.
    fn b3_as_traceparent(&self) -> Option<String> {
        // B3 single: `traceid-spanid[-sampled[-parentspanid]]`
        let (trace_id, span_id, sampled) = if let Some(b3) = self.get("b3") {
            let mut parts = b3.split('-');
            (
                parts.next().unwrap_or_default().to_string(),
                parts.next().unwrap_or_default().to_string(),
                parts.next().map(str::to_string),
            )
        } else {
            (
                self.get("x-b3-traceid")?.to_string(),
                self.get("x-b3-spanid")?.to_string(),
                self.get("x-b3-sampled").map(str::to_string),
            )
        };

        // a 64-bit B3 trace id is the low half of a 128-bit one
        let trace_id = match trace_id.len() {
            32 if is_hex(&trace_id) => trace_id.to_lowercase(),
            16 if is_hex(&trace_id) => format!("{:0>32}", trace_id.to_lowercase()),
            _ => return None,
        };
        if span_id.len() != 16 || !is_hex(&span_id) {
            return None;
        }
        // `d` (debug) implies sampled; absent defaults to sampled so a caller
        // that omitted the flag is not silently dropped from the trace
        let flags = match sampled.as_deref() {
            Some("0") | Some("false") => "00",
            _ => "01",
        };
        Some(format!("00-{trace_id}-{}-{flags}", span_id.to_lowercase()))
    }
}

fn is_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Environment variable that hard-disables every telemetry export.
pub const TELEMETRY_ENABLED_ENV: &str = "ROLTER_TELEMETRY_ENABLED";

/// Whether telemetry export is permitted at all (#812).
///
/// Returns `false` only when [`TELEMETRY_ENABLED_ENV`] is set to an explicit
/// falsy value. Unset means enabled, so an existing deployment is unaffected —
/// "off" remains the default in practice because nothing is exported until an
/// OTLP endpoint is configured.
///
/// This is a *kill switch*, not a second way to turn telemetry on: it can only
/// subtract. When false, no exporter is built regardless of which `OTEL_*`
/// endpoints are set, which is the point — "off" was previously implicit
/// (achieved by leaving an endpoint unset), so it did not survive somebody
/// setting the endpoint for one signal and there was nothing to point at in a
/// security review.
///
/// Deliberately environment-only rather than a config-file key. `init` installs
/// the subscriber in `main` before any config file is read, so a config-file
/// switch could not gate trace export at all, and honouring it only for the
/// signals initialized later would mean the same key meant different things
/// depending on which signal you asked about. The `OTEL_*` contract this
/// composes with is environment-based for the same reason.
#[must_use]
pub fn export_enabled() -> bool {
    match std::env::var(TELEMETRY_ENABLED_ENV) {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Guard returned by [`init`]; holds the OTLP tracer provider (when configured)
/// so spans are flushed to the collector on process exit.
///
/// Bind it to a named local in `main` (`let _telemetry = telemetry::init();`) —
/// binding to `_` would drop it immediately and discard buffered spans.
#[must_use = "bind the guard to a named local so spans flush on exit"]
#[derive(Default)]
pub struct TelemetryGuard {
    #[cfg(feature = "otlp")]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otlp")]
        if let Some(provider) = self.provider.take() {
            // flush any batched spans before the runtime tears down
            let _ = provider.shutdown();
        }
    }
}

/// Initialize the global tracing subscriber.
///
/// Reads the `RUST_LOG` environment variable for log filtering and falls back to
/// `info`. When the `otlp` feature is enabled (default) and an OTLP endpoint is
/// configured via the standard `OTEL_EXPORTER_OTLP_ENDPOINT` /
/// `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` environment variable, spans are also
/// exported to that OpenTelemetry collector; otherwise only the stdout fmt layer
/// is installed and there is no OTLP overhead.
///
/// Safe to call more than once; subsequent calls are ignored.
pub fn init() -> TelemetryGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    #[cfg(feature = "otlp")]
    if let Some(provider) = otlp::try_build_provider() {
        use opentelemetry::trace::TracerProvider as _;
        let tracer = provider.tracer("rolter");
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer())
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .try_init();
        // the W3C propagator is what makes an inbound `traceparent` become a
        // real parent and an outbound one carry *this* gateway's span; without
        // it every span is a disconnected root
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );
        PIPELINE_ACTIVE.store(true, Ordering::Relaxed);
        return TelemetryGuard {
            provider: Some(provider),
        };
    }

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .try_init();
    TelemetryGuard::default()
}

/// One scalar metric to export: Prometheus type, name, help text, value.
///
/// Mirrors what the gateway's `Metrics::scalars()` produces, expressed in plain
/// types so `rolter-core` needs no dependency on the gateway.
pub type ScalarMetric = (&'static str, &'static str, &'static str, u64);

/// One series of a labelled counter: `(family name, attributes, value)`.
///
/// Separate from [`ScalarMetric`] because the *set of series* is discovered at
/// runtime — a model, target or variant only exists once traffic has used it —
/// while the family it belongs to is fixed. The attribute values are owned
/// because they come from `DashMap` keys that may change under the collection.
pub type LabelledMetric = (&'static str, Vec<(&'static str, String)>, u64);

/// A labelled counter family: `(name, help)`.
///
/// Declared up front rather than inferred from the first collection, because at
/// install time a gateway has served nothing and every family is legitimately
/// empty. Inferring would silently export nothing until a restart.
pub type LabelledFamily = (&'static str, &'static str);

/// What the gateway hands the exporter. A struct rather than four positional
/// arguments so the call site says which is which.
pub struct MetricsExport<S, L> {
    /// Unlabelled counters and gauges — the same list the Prometheus endpoint renders.
    pub scalars: S,
    /// Fixed set of labelled counter families.
    pub labelled_families: &'static [LabelledFamily],
    /// Every currently-known series across those families.
    pub labelled: L,
    /// Explicit histogram bucket boundaries, in milliseconds.
    ///
    /// Passed in rather than defaulted so the OTLP histogram and the Prometheus
    /// one share boundaries. Two exporters of the same measurement disagreeing
    /// about where the buckets fall is worse than either alone, because the
    /// numbers look comparable and are not.
    pub latency_buckets_ms: Vec<f64>,
}

/// Records per-request latency and time-to-first-token as OTLP histograms.
///
/// Unlike the counters, this cannot be an observable instrument: OTel builds a
/// histogram from individual measurements, and the gateway's pre-bucketed
/// counters cannot be handed over after the fact. So this is the one part of
/// metrics export that touches the request path — and it does so only when an
/// OTLP endpoint is configured.
///
/// A default-constructed value records nothing, which is what every deployment
/// without OTLP holds. Cloning is cheap; the instruments are shared.
#[derive(Clone, Default)]
pub struct RequestHistograms {
    #[cfg(feature = "otlp")]
    inner: Option<std::sync::Arc<HistogramSet>>,
}

#[cfg(feature = "otlp")]
struct HistogramSet {
    latency: opentelemetry::metrics::Histogram<u64>,
    ttft: opentelemetry::metrics::Histogram<u64>,
}

impl RequestHistograms {
    /// Record one completed request against the `model` attribute.
    ///
    /// Does nothing, and allocates nothing, when metrics export is off.
    pub fn record(&self, model: &str, latency_ms: u32, ttft_ms: u32) {
        #[cfg(feature = "otlp")]
        if let Some(inner) = &self.inner {
            // one attribute set for both instruments — `model` is the same
            // bounded label the Prometheus histogram already uses, so this adds
            // no cardinality the deployment was not already carrying
            let attrs = [opentelemetry::KeyValue::new("model", model.to_string())];
            inner.latency.record(u64::from(latency_ms), &attrs);
            inner.ttft.record(u64::from(ttft_ms), &attrs);
            return;
        }
        let _ = (model, latency_ms, ttft_ms);
    }

    /// Whether anything is actually being recorded.
    #[must_use]
    pub fn is_active(&self) -> bool {
        // a `let` per feature rather than a cfg'd `return`, which clippy reads
        // as a needless return on the branch that is actually compiled
        #[cfg(feature = "otlp")]
        let active = self.inner.is_some();
        #[cfg(not(feature = "otlp"))]
        let active = false;
        active
    }
}

/// Guard holding the OTLP meter provider, flushing metrics on drop.
#[must_use = "bind the guard to a named local so metrics flush on exit"]
#[derive(Default)]
pub struct MetricsGuard {
    #[cfg(feature = "otlp")]
    provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    histograms: RequestHistograms,
}

impl MetricsGuard {
    /// The histogram recorder to hand the request path. Inert when export is off.
    pub fn histograms(&self) -> RequestHistograms {
        self.histograms.clone()
    }
}

impl Drop for MetricsGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otlp")]
        if let Some(provider) = self.provider.take() {
            let _ = provider.shutdown();
        }
    }
}

/// Export the gateway's existing counters and gauges over OTLP (#805).
///
/// A *second exporter over the same numbers*, not a rewrite: `collect` hands
/// back whatever the Prometheus endpoint would render, and both stay in step
/// because they read one list. The Prometheus endpoint is untouched.
///
/// Instruments are observable — nothing is pushed on the request path. The SDK
/// invokes `collect` on its own periodic interval (`OTEL_METRIC_EXPORT_INTERVAL`,
/// default 60s), so the hot path keeps doing nothing but `fetch_add` on an
/// atomic.
///
/// Returns `None` when no OTLP endpoint is configured, leaving the untraced
/// deployment exactly as it was.
pub fn install_metrics<S, L>(export: MetricsExport<S, L>) -> Option<MetricsGuard>
where
    S: Fn() -> Vec<ScalarMetric> + Send + Sync + Clone + 'static,
    L: Fn() -> Vec<LabelledMetric> + Send + Sync + Clone + 'static,
{
    #[cfg(feature = "otlp")]
    {
        let MetricsExport {
            scalars: collect,
            labelled_families,
            labelled,
            latency_buckets_ms,
        } = export;
        let provider = otlp::try_build_meter_provider()?;
        let meter = {
            use opentelemetry::metrics::MeterProvider as _;
            provider.meter("rolter")
        };

        // the instrument set is fixed at install time from one snapshot; each
        // instrument then re-reads its own value by name on every collection
        for (kind, name, help, _) in collect() {
            let pick = collect.clone();
            let observe = move |value: &dyn Fn(u64)| {
                if let Some((_, _, _, v)) = pick().into_iter().find(|(_, n, _, _)| *n == name) {
                    value(v);
                }
            };
            match kind {
                // a gauge can go down; a counter is monotonic. exporting a
                // counter as a gauge would break rate() on the backend
                "gauge" => {
                    let o = observe.clone();
                    meter
                        .u64_observable_gauge(name)
                        .with_description(help)
                        .with_callback(move |obs| o(&|v| obs.observe(v, &[])))
                        .build();
                }
                _ => {
                    let o = observe.clone();
                    meter
                        .u64_observable_counter(name)
                        .with_description(help)
                        .with_callback(move |obs| o(&|v| obs.observe(v, &[])))
                        .build();
                }
            }
        }

        // one observable counter per family; the callback emits every series
        // that family currently holds. registering per-series instead would
        // freeze the set at install time, when the gateway has served nothing
        for (family, help) in labelled_families {
            let pick = labelled.clone();
            meter
                .u64_observable_counter(*family)
                .with_description(*help)
                .with_callback(move |obs| {
                    for (name, attrs, value) in pick() {
                        if name != *family {
                            continue;
                        }
                        let kv: Vec<opentelemetry::KeyValue> = attrs
                            .into_iter()
                            .map(|(k, v)| opentelemetry::KeyValue::new(k, v))
                            .collect();
                        obs.observe(value, &kv);
                    }
                })
                .build();
        }

        // histograms are the one instrument that cannot be observable: OTel
        // builds them from individual measurements, so the gateway's
        // pre-bucketed counters cannot be handed over after the fact
        let histograms = RequestHistograms {
            inner: Some(std::sync::Arc::new(HistogramSet {
                latency: meter
                    .u64_histogram("rolter_request_latency_ms")
                    .with_description("total request latency in milliseconds")
                    .with_unit("ms")
                    .with_boundaries(latency_buckets_ms.clone())
                    .build(),
                ttft: meter
                    .u64_histogram("rolter_request_ttft_ms")
                    .with_description("time to first token in milliseconds")
                    .with_unit("ms")
                    .with_boundaries(latency_buckets_ms)
                    .build(),
            })),
        };

        Some(MetricsGuard {
            provider: Some(provider),
            histograms,
        })
    }
    #[cfg(not(feature = "otlp"))]
    {
        let _ = export;
        None
    }
}

/// Build a span for one pipeline stage, or a disabled span when no OTLP
/// pipeline is installed (#805).
///
/// Stage spans are what make a slow request attributable to `auth`,
/// `route.select`, `queue.wait` or `upstream.request` rather than merely slow.
/// They are worth nothing to a deployment that exports no traces, so this
/// creates none there: [`tracing::Span::none`] allocates nothing, and
/// instrumenting a future with it is a no-op — the untraced hot path stays as it
/// was.
///
/// Takes the same arguments as [`tracing::info_span!`]. Hold the returned span
/// for the length of the stage and drop it at the end; do not call `.entered()`
/// on it inside an async fn, as the resulting guard is `!Send` and would make
/// the whole future `!Send`. Use `.instrument(span)` to cover an `await`.
#[macro_export]
macro_rules! stage_span {
    ($name:literal $(, $($field:tt)*)?) => {
        if $crate::telemetry::is_active() {
            tracing::info_span!($name $(, $($field)*)?)
        } else {
            tracing::Span::none()
        }
    };
}

/// Adopt the caller's inbound trace context as the parent of `span`.
///
/// Turns the gateway's request span from a disconnected root into a child of
/// whatever called it, so a dashboard click and the provider request it caused
/// land in one trace. A no-op when no pipeline is installed, when the carrier is
/// empty, or when the context it carries is malformed.
pub fn adopt_inbound(span: &tracing::Span, carrier: &TraceCarrier) {
    #[cfg(feature = "otlp")]
    {
        if !is_active() || carrier.is_empty() {
            return;
        }
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let parent = opentelemetry::global::get_text_map_propagator(|p| p.extract(carrier));
        // a span with no parent is better than a failed request: a malformed
        // inbound context is dropped, never propagated
        let _ = span.set_parent(parent);
    }
    #[cfg(not(feature = "otlp"))]
    {
        let _ = (span, carrier);
    }
}

/// Inject the *current* span's trace context as outbound headers.
///
/// This is what makes the upstream provider call a child of the gateway span
/// rather than a sibling: the emitted `traceparent` names this gateway's span as
/// the parent, not the caller's. Returns an empty vec when no pipeline is
/// installed or the current span has no valid context, so an untraced
/// deployment puts nothing extra on the wire.
#[must_use]
pub fn inject_current() -> Vec<(String, String)> {
    #[cfg(feature = "otlp")]
    {
        if !is_active() {
            return Vec::new();
        }
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let cx = tracing::Span::current().context();
        let mut sink = HeaderSink::default();
        opentelemetry::global::get_text_map_propagator(|p| p.inject_context(&cx, &mut sink));
        sink.0
    }
    #[cfg(not(feature = "otlp"))]
    {
        Vec::new()
    }
}

/// Trace id of the current span as lowercase hex, or `None` when there is no
/// valid context.
///
/// Preferred over re-parsing the inbound header: once a parent is adopted this
/// is the same id the exported spans carry, so the ClickHouse request log and
/// the trace backend agree by construction rather than by coincidence.
#[must_use]
pub fn current_trace_id() -> Option<String> {
    #[cfg(feature = "otlp")]
    {
        if !is_active() {
            return None;
        }
        use opentelemetry::trace::TraceContextExt as _;
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let cx = tracing::Span::current().context();
        let span_cx = cx.span().span_context().clone();
        span_cx
            .is_valid()
            .then(|| span_cx.trace_id().to_string().to_lowercase())
    }
    #[cfg(not(feature = "otlp"))]
    {
        None
    }
}

#[cfg(feature = "otlp")]
impl opentelemetry::propagation::Extractor for TraceCarrier {
    fn get(&self, key: &str) -> Option<&str> {
        TraceCarrier::get(self, key)
    }

    fn keys(&self) -> Vec<&str> {
        self.entries.iter().map(|(k, _)| k.as_str()).collect()
    }
}

/// Collects the headers a propagator injects.
#[cfg(feature = "otlp")]
#[derive(Default)]
struct HeaderSink(Vec<(String, String)>);

#[cfg(feature = "otlp")]
impl opentelemetry::propagation::Injector for HeaderSink {
    fn set(&mut self, key: &str, value: String) {
        self.0.push((key.to_string(), value));
    }
}

#[cfg(test)]
mod kill_switch_tests {
    use super::*;

    /// `export_enabled` reads process-global env, so the cases that set it must
    /// not interleave with each other or with the pipeline tests.
    fn with_env(value: Option<&str>, f: impl FnOnce()) {
        let _guard = super::pipeline_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match value {
            Some(v) => std::env::set_var(TELEMETRY_ENABLED_ENV, v),
            None => std::env::remove_var(TELEMETRY_ENABLED_ENV),
        }
        f();
        std::env::remove_var(TELEMETRY_ENABLED_ENV);
    }

    #[test]
    fn unset_means_enabled_so_an_existing_deployment_is_unaffected() {
        with_env(None, || assert!(export_enabled()));
    }

    #[test]
    fn every_falsy_spelling_disables_export() {
        for value in ["0", "false", "FALSE", "no", "off", " Off "] {
            with_env(Some(value), || {
                assert!(!export_enabled(), "{value} should disable export");
            });
        }
    }

    #[test]
    fn truthy_and_unrecognized_values_leave_export_on() {
        // the switch can only subtract: it is not a second way to turn export
        // on, and an unparseable value must not silently blind a deployment
        for value in ["1", "true", "yes", "on", "maybe", ""] {
            with_env(Some(value), || {
                assert!(export_enabled(), "{value} should leave export on");
            });
        }
    }

    #[cfg(feature = "otlp")]
    #[test]
    fn the_switch_beats_a_configured_endpoint() {
        // the whole point of #812: "off" must not depend on remembering to
        // leave every per-signal endpoint unset
        let _guard = super::pipeline_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4317");
        std::env::set_var(TELEMETRY_ENABLED_ENV, "false");

        assert!(super::otlp::try_build_provider().is_none());
        assert!(super::otlp::try_build_meter_provider().is_none());

        std::env::remove_var(TELEMETRY_ENABLED_ENV);
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    }
}

#[cfg(test)]
mod carrier_tests {
    use super::*;

    fn carrier(pairs: &[(&str, &str)]) -> TraceCarrier {
        let mut c = TraceCarrier::new();
        for (k, v) in pairs {
            c.insert(k, v);
        }
        c.finish()
    }

    #[test]
    fn w3c_traceparent_is_kept_as_sent() {
        let c = carrier(&[(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )]);
        assert_eq!(
            c.get("traceparent"),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        );
    }

    #[test]
    fn b3_single_becomes_a_traceparent() {
        let c = carrier(&[("b3", "80f198ee56343ba864fe8b2a57d3eff7-e457b5a2e4d86bd1-1")]);
        assert_eq!(
            c.get("traceparent"),
            Some("00-80f198ee56343ba864fe8b2a57d3eff7-e457b5a2e4d86bd1-01")
        );
    }

    #[test]
    fn b3_multi_becomes_a_traceparent() {
        let c = carrier(&[
            ("X-B3-TraceId", "80f198ee56343ba864fe8b2a57d3eff7"),
            ("X-B3-SpanId", "e457b5a2e4d86bd1"),
            ("X-B3-Sampled", "0"),
        ]);
        assert_eq!(
            c.get("traceparent"),
            Some("00-80f198ee56343ba864fe8b2a57d3eff7-e457b5a2e4d86bd1-00")
        );
    }

    #[test]
    fn a_64_bit_b3_trace_id_is_left_padded_to_128_bits() {
        let c = carrier(&[("b3", "a3ce929d0e0e4736-e457b5a2e4d86bd1-1")]);
        assert_eq!(
            c.get("traceparent"),
            Some("00-0000000000000000a3ce929d0e0e4736-e457b5a2e4d86bd1-01")
        );
    }

    #[test]
    fn an_inbound_traceparent_wins_over_b3() {
        let c = carrier(&[
            (
                "traceparent",
                "00-11111111111111111111111111111111-2222222222222222-01",
            ),
            ("b3", "80f198ee56343ba864fe8b2a57d3eff7-e457b5a2e4d86bd1-1"),
        ]);
        assert_eq!(
            c.get("traceparent"),
            Some("00-11111111111111111111111111111111-2222222222222222-01")
        );
    }

    #[test]
    fn malformed_and_blank_context_is_dropped() {
        // non-hex, wrong length, and a b3 with no span id all yield nothing —
        // a bad inbound context must never synthesize a bogus parent
        assert!(carrier(&[("b3", "nothex-e457b5a2e4d86bd1-1")])
            .get("traceparent")
            .is_none());
        assert!(carrier(&[("b3", "80f198ee56343ba864fe8b2a57d3eff7")])
            .get("traceparent")
            .is_none());
        assert!(carrier(&[("traceparent", "")]).is_empty());
    }

    #[test]
    fn the_no_otlp_path_extracts_and_injects_nothing() {
        // the bar the issue sets: with no pipeline installed, propagation is
        // inert and costs nothing beyond an atomic load
        let _guard = super::pipeline_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(!is_active());
        assert!(inject_current().is_empty());
        assert!(current_trace_id().is_none());
    }
}

/// Serializes the tests that flip the global `PIPELINE_ACTIVE` flag against the
/// ones that assert it is off, since cargo runs them on parallel threads in one
/// process.
#[cfg(test)]
fn pipeline_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(all(test, feature = "otlp"))]
mod propagation_tests {
    // `super::*` already carries `tracing_subscriber::prelude::*`
    use super::*;

    const INBOUND_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const INBOUND_SPAN: &str = "00f067aa0ba902b7";

    fn inbound_traceparent() -> String {
        format!("00-{INBOUND_TRACE}-{INBOUND_SPAN}-01")
    }

    /// Run `f` with a real tracer provider and the W3C propagator installed, so
    /// spans get valid contexts.
    fn with_pipeline<T>(f: impl FnOnce() -> T) -> T {
        use opentelemetry::trace::TracerProvider as _;

        let _guard = super::pipeline_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let tracer = provider.tracer("test");
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );
        PIPELINE_ACTIVE.store(true, Ordering::Relaxed);

        let subscriber =
            tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
        let out = tracing::subscriber::with_default(subscriber, f);

        PIPELINE_ACTIVE.store(false, Ordering::Relaxed);
        out
    }

    /// Split a `traceparent` into its trace id and parent span id.
    fn parts(traceparent: &str) -> (String, String) {
        let fields: Vec<&str> = traceparent.split('-').collect();
        assert_eq!(fields.len(), 4, "malformed traceparent: {traceparent}");
        (fields[1].to_string(), fields[2].to_string())
    }

    #[test]
    fn the_upstream_call_parents_to_the_gateway_span_not_the_caller() {
        // this is the #805 bug in one assertion. the forwarder used to copy the
        // inbound `traceparent` verbatim, so the provider call named the
        // *caller's* span as its parent and came out a sibling of the gateway's
        // own work — every waterfall built from that data was wrong. the
        // injected context must keep the trace id and replace the span id
        let injected = with_pipeline(|| {
            let span = tracing::info_span!("gateway.request");
            let mut carrier = TraceCarrier::new();
            carrier.insert("traceparent", &inbound_traceparent());
            adopt_inbound(&span, &carrier.finish());

            let _entered = span.enter();
            inject_current()
        });

        let traceparent = injected
            .iter()
            .find(|(k, _)| k == "traceparent")
            .map(|(_, v)| v.as_str())
            .expect("a traceparent must be injected");
        let (trace_id, parent_span_id) = parts(traceparent);

        // same trace: the caller and the provider call are one story
        assert_eq!(
            trace_id, INBOUND_TRACE,
            "the inbound trace must be continued, not restarted"
        );
        // different span: the gateway's span is the parent now, not the caller's
        assert_ne!(
            parent_span_id, INBOUND_SPAN,
            "the upstream leg must parent to the gateway span, not to the caller's"
        );
    }

    #[test]
    fn the_logged_trace_id_matches_the_exported_span() {
        // the request log and the trace backend must agree by construction
        let (trace_id, injected) = with_pipeline(|| {
            let span = tracing::info_span!("gateway.request");
            let mut carrier = TraceCarrier::new();
            carrier.insert("traceparent", &inbound_traceparent());
            adopt_inbound(&span, &carrier.finish());

            let _entered = span.enter();
            (current_trace_id(), inject_current())
        });

        assert_eq!(trace_id.as_deref(), Some(INBOUND_TRACE));
        let traceparent = injected
            .iter()
            .find(|(k, _)| k == "traceparent")
            .map(|(_, v)| v.as_str())
            .expect("a traceparent must be injected");
        assert_eq!(parts(traceparent).0, INBOUND_TRACE);
    }

    #[test]
    fn a_b3_only_caller_is_continued_too() {
        // the gateway already accepted B3 inbound; adopting it must keep working
        // now that a W3C propagator does the extraction
        let trace_id = with_pipeline(|| {
            let span = tracing::info_span!("gateway.request");
            let mut carrier = TraceCarrier::new();
            carrier.insert("b3", &format!("{INBOUND_TRACE}-{INBOUND_SPAN}-1"));
            adopt_inbound(&span, &carrier.finish());

            let _entered = span.enter();
            current_trace_id()
        });

        assert_eq!(trace_id.as_deref(), Some(INBOUND_TRACE));
    }

    #[test]
    fn an_untraced_caller_starts_a_fresh_trace_rather_than_failing() {
        // no inbound context at all: the gateway span is a root, and what it
        // injects is still valid so the provider call hangs off it
        let injected = with_pipeline(|| {
            let span = tracing::info_span!("gateway.request");
            adopt_inbound(&span, &TraceCarrier::new().finish());

            let _entered = span.enter();
            inject_current()
        });

        let traceparent = injected
            .iter()
            .find(|(k, _)| k == "traceparent")
            .map(|(_, v)| v.as_str())
            .expect("a traceparent must be injected");
        let (trace_id, parent_span_id) = parts(traceparent);
        assert_ne!(trace_id, INBOUND_TRACE);
        assert_ne!(trace_id, "0".repeat(32));
        assert_ne!(parent_span_id, "0".repeat(16));
    }
}

#[cfg(feature = "otlp")]
mod otlp {
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use opentelemetry_sdk::Resource;

    /// Build an OTLP tracer provider from the standard `OTEL_*` environment, or
    /// `None` when no OTLP endpoint is configured (tracing stays stdout-only).
    ///
    /// Honours the OpenTelemetry SDK env contract: `OTEL_EXPORTER_OTLP_ENDPOINT`
    /// (or the traces-specific `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`) selects the
    /// receiver, `OTEL_EXPORTER_OTLP_PROTOCOL` picks gRPC (default) vs
    /// HTTP/protobuf, `OTEL_EXPORTER_OTLP_HEADERS` carries backend auth, and
    /// `OTEL_SERVICE_NAME` names the service (default `rolter`).
    pub fn try_build_provider() -> Option<SdkTracerProvider> {
        // the kill switch wins over every endpoint setting (#812)
        if !super::export_enabled() {
            return None;
        }
        // only wire the exporter when an endpoint is set; keeps the default path
        // (no env) allocation- and network-free
        if std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_none()
            && std::env::var_os("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").is_none()
        {
            return None;
        }

        // endpoint, headers and timeout are read from the OTEL_* env by the
        // exporter builder; we only steer the transport here
        let protocol = std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL").unwrap_or_default();
        let exporter = if protocol.starts_with("http") {
            SpanExporter::builder().with_http().build()
        } else {
            SpanExporter::builder().with_tonic().build()
        };
        let exporter = match exporter {
            Ok(exporter) => exporter,
            Err(err) => {
                // a misconfigured collector must not take the gateway down; log
                // and fall back to stdout-only tracing
                eprintln!("rolter: OTLP span exporter init failed, tracing stays local: {err}");
                return None;
            }
        };

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(resource())
            .build();
        opentelemetry::global::set_tracer_provider(provider.clone());
        Some(provider)
    }

    /// Build an OTLP meter provider from the same `OTEL_*` environment the
    /// tracer uses, or `None` when no endpoint is configured.
    pub fn try_build_meter_provider() -> Option<opentelemetry_sdk::metrics::SdkMeterProvider> {
        // one switch covers every signal, so metrics go quiet with traces (#812)
        if !super::export_enabled() {
            return None;
        }
        if std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_none()
            && std::env::var_os("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT").is_none()
        {
            return None;
        }

        let protocol = std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL").unwrap_or_default();
        let exporter = if protocol.starts_with("http") {
            opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .build()
        } else {
            opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .build()
        };
        let exporter = match exporter {
            Ok(exporter) => exporter,
            Err(err) => {
                // as with traces: a bad collector must not take the gateway down
                eprintln!("rolter: OTLP metric exporter init failed, metrics stay local: {err}");
                return None;
            }
        };

        Some(
            opentelemetry_sdk::metrics::SdkMeterProvider::builder()
                .with_periodic_exporter(exporter)
                .with_resource(resource())
                .build(),
        )
    }

    fn resource() -> Resource {
        let service = std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "rolter".to_string());
        Resource::builder().with_service_name(service).build()
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn no_endpoint_means_no_exporter() {
            // the default path (no OTLP endpoint configured) must stay
            // exporter-free so tracing has zero network overhead
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
            std::env::remove_var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT");
            assert!(super::try_build_provider().is_none());
        }
    }
}
