//! Request correlation: an end-to-end `x-request-id` and inbound distributed-
//! trace continuation (ROL-60).
//!
//! [`ensure_request_id`] is a middleware that guarantees every request carries an
//! `x-request-id` — reusing the caller's when present, else minting a UUID — and
//! echoes it on the response so a client can correlate its call with the gateway
//! logs. [`inbound_trace_id`] pulls the trace id out of a W3C `traceparent` or a
//! B3 header so the request log adopts the caller's trace instead of starting a
//! disconnected one; the id is stored on each [`RequestLog`](crate::logging::RequestLog)
//! and surfaces in ClickHouse for cross-service correlation.
//!
//! [`GatewayMakeSpan`] takes that further (#805): it builds the request span and
//! makes the inbound context its real OpenTelemetry *parent*, and [`outbound_headers`]
//! injects the gateway's own span into the upstream call rather than copying the
//! caller's header verbatim. Together those are what put a dashboard click, the
//! gateway, and the provider request in one correctly-parented trace. All of it
//! is inert unless an OTLP endpoint is configured.

use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tower_http::trace::{MakeSpan, OnResponse};
use tracing::Span;

/// header carrying the end-to-end request id
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Pipeline-stage span, defined in `rolter-core` so the proxy's translation
/// stages use the same one. Re-exported here because that is where the gateway
/// reaches for tracing helpers.
pub(crate) use rolter_core::stage_span;

/// Ensure the request has an `x-request-id` (generating one when absent or
/// blank), expose it to downstream handlers via the request headers, and mirror
/// it onto the response.
pub async fn ensure_request_id(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(new_request_id);

    // a generated uuid is always a valid header value; a caller-supplied one that
    // isn't is dropped rather than failing the request
    let Ok(header) = HeaderValue::from_str(&id) else {
        return next.run(req).await;
    };
    req.headers_mut().insert(REQUEST_ID_HEADER, header.clone());
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(REQUEST_ID_HEADER, header);
    resp
}

fn new_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Builds the per-request span and adopts the caller's inbound trace context as
/// its parent (#805).
///
/// This has to be the [`TraceLayer`](tower_http::trace::TraceLayer)'s own
/// span-maker rather than a middleware layered inside it. `DefaultMakeSpan`
/// creates the request span at **DEBUG**, so under the default `RUST_LOG=info`
/// it is disabled — and setting a parent on a disabled span does nothing, which
/// left the stage spans as disconnected roots carrying a freshly-minted trace id
/// even though the caller had sent a perfectly good `traceparent`. Building the
/// span here, at `INFO`, is what makes it real enough to parent and to export.
///
/// When no OTLP pipeline is installed it falls back to the stock DEBUG span, so
/// an untraced deployment keeps paying exactly what it paid before: the span is
/// disabled by the default filter and never allocated.
#[derive(Clone, Copy, Debug, Default)]
pub struct GatewayMakeSpan;

impl<B> MakeSpan<B> for GatewayMakeSpan {
    fn make_span(&mut self, req: &axum::http::Request<B>) -> Span {
        if !rolter_core::telemetry::is_active() {
            // stock behaviour: disabled at the default filter, costs nothing
            return tracing::debug_span!(
                "request",
                method = %req.method(),
                uri = %req.uri(),
                version = ?req.version(),
            );
        }

        // `otel.name` is what a backend shows as the span name; method + route
        // reads better in a waterfall than the bare path
        // `otel.kind=server` marks this as the inbound leg. rolter is a server
        // in the GenAI conventions' sense, and the kind was previously never
        // set at all — `otel.kind` appeared nowhere in the gateway (#808)
        let span = tracing::info_span!(
            "gateway.request",
            otel.name = %format!("{} {}", req.method(), req.uri().path()),
            otel.kind = "server",
            http.request.method = %req.method(),
            url.path = %req.uri().path(),
        );

        let mut carrier = rolter_core::telemetry::TraceCarrier::new();
        for name in PROPAGATED_TRACE_HEADERS {
            if let Some(value) = req.headers().get(*name).and_then(|v| v.to_str().ok()) {
                carrier.insert(name, value);
            }
        }
        rolter_core::telemetry::adopt_inbound(&span, &carrier.finish());
        span
    }
}

/// The trace id to stamp on this request's [`RequestLog`](crate::logging::RequestLog).
///
/// Prefers the id of the span actually being exported, so ClickHouse and the
/// trace backend agree by construction; falls back to parsing the inbound
/// header when no pipeline is installed, which is the pre-#805 behaviour and
/// the only path an untraced deployment takes.
pub fn request_trace_id(headers: &HeaderMap) -> String {
    rolter_core::telemetry::current_trace_id().unwrap_or_else(|| inbound_trace_id(headers))
}

/// [`TraceLayer`](tower_http::trace::TraceLayer) response hook that surfaces
/// failing requests on the terminal at the default `info` filter.
///
/// The stock `DefaultOnResponse` logs every response at `DEBUG`, so with the
/// default `RUST_LOG=info` an operator running `rolter`/`uvx rolter` never sees
/// 4xx/5xx responses (ROL-230): a 404 for an unknown model, a 401 for a bad key
/// or a 502 from a dead upstream all vanish unless `RUST_LOG` is turned up. This
/// hook picks the level from the status class instead — server errors at
/// `error`, client errors at `warn`, everything else at `debug` — so errors are
/// visible out of the box while successful traffic stays quiet. Pair it with
/// `.on_failure(())` on the layer so classified 5xx responses are not also
/// logged by the default failure hook.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatusAwareOnResponse;

/// Pick the log level (and its message) for a response status class: server
/// errors are loud, client errors are warnings, success is quiet.
fn level_for_status(status: axum::http::StatusCode) -> (tracing::Level, &'static str) {
    if status.is_server_error() {
        (tracing::Level::ERROR, "request failed")
    } else if status.is_client_error() {
        (tracing::Level::WARN, "request rejected")
    } else {
        (tracing::Level::DEBUG, "request completed")
    }
}

impl<B> OnResponse<B> for StatusAwareOnResponse {
    fn on_response(self, response: &axum::http::Response<B>, latency: Duration, _span: &Span) {
        let status = response.status();
        let latency_ms = latency.as_millis();
        let request_id = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // the message + level are fixed per status class; the macro must still be
        // invoked per-branch because the level is a compile-time argument
        match level_for_status(status).0 {
            tracing::Level::ERROR => {
                tracing::error!(
                    status = status.as_u16(),
                    latency_ms,
                    request_id,
                    "request failed"
                )
            }
            tracing::Level::WARN => {
                tracing::warn!(
                    status = status.as_u16(),
                    latency_ms,
                    request_id,
                    "request rejected"
                )
            }
            _ => {
                tracing::debug!(
                    status = status.as_u16(),
                    latency_ms,
                    request_id,
                    "request completed"
                )
            }
        }
    }
}

/// Extract an inbound trace id from a W3C `traceparent`, a B3 single header, or
/// the `x-b3-traceid` multi-header, normalized to lowercase hex. Returns an
/// empty string when no well-formed trace id is present.
pub fn inbound_trace_id(headers: &HeaderMap) -> String {
    // W3C traceparent: `version-traceid-spanid-flags`; trace id is 32 hex chars
    if let Some(tp) = headers.get("traceparent").and_then(|v| v.to_str().ok()) {
        let parts: Vec<&str> = tp.split('-').collect();
        if parts.len() >= 3 && is_hex(parts[1], 32) {
            return parts[1].to_lowercase();
        }
    }
    // B3 single header: `traceid-spanid[-sampled[-parentspanid]]` (64- or 128-bit)
    if let Some(b3) = headers.get("b3").and_then(|v| v.to_str().ok()) {
        let first = b3.split('-').next().unwrap_or("");
        if is_hex(first, 32) || is_hex(first, 16) {
            return first.to_lowercase();
        }
    }
    // B3 multi header
    if let Some(tid) = headers.get("x-b3-traceid").and_then(|v| v.to_str().ok()) {
        if is_hex(tid, 32) || is_hex(tid, 16) {
            return tid.to_lowercase();
        }
    }
    String::new()
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// standard distributed-trace headers propagated verbatim to the upstream
const PROPAGATED_TRACE_HEADERS: &[&str] = &[
    "traceparent",
    "tracestate",
    "b3",
    "x-b3-traceid",
    "x-b3-spanid",
    "x-b3-sampled",
    "x-b3-parentspanid",
    "x-b3-flags",
];

/// Collect the caller's inbound trace-context headers so the forwarder can
/// propagate them verbatim to the upstream, letting vLLM/SGLang/TGI continue the
/// same trace (ROL-61). Returns an empty vec when the caller sent none, so an
/// untraced request adds nothing to the wire.
pub fn outbound_trace_headers(headers: &HeaderMap) -> Vec<(&'static str, String)> {
    PROPAGATED_TRACE_HEADERS
        .iter()
        .filter_map(|&name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .filter(|s| !s.is_empty())
                .map(|v| (name, v.to_string()))
        })
        .collect()
}

/// Trace context injected from the current span, or `None` when no OTLP
/// pipeline is installed or the current span has no valid context — in which
/// case the caller falls back to copying the inbound headers.
fn injected_trace_headers() -> Option<Vec<(String, String)>> {
    if !rolter_core::telemetry::is_active() {
        return None;
    }
    let injected = rolter_core::telemetry::inject_current();
    (!injected.is_empty()).then_some(injected)
}

/// Trace context plus the operator-configured client headers, ready to hand to
/// the forwarder (#564).
///
/// Trace headers always propagate and always win: an allowlist entry naming one
/// of them cannot replace the context the gateway resolved, so a misconfigured
/// list can never break correlation.
///
/// With an OTLP pipeline installed the trace context is *injected from the
/// current span* rather than copied from the caller (#805). That is the
/// difference between the provider call being a child of the gateway span and
/// being its sibling — copying the inbound header verbatim parented the upstream
/// leg to the caller, which made every waterfall built from the data wrong. Call
/// this from inside the span the upstream request should hang off.
///
/// With no pipeline installed it copies the caller's headers verbatim, exactly
/// as before.
pub fn outbound_headers(headers: &HeaderMap, forwarded: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = injected_trace_headers().unwrap_or_else(|| {
        outbound_trace_headers(headers)
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect()
    });
    for name in forwarded {
        if PROPAGATED_TRACE_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if let Some(value) = headers
            .get(name.as_str())
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
        {
            out.push((name.clone(), value.to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn parses_w3c_traceparent() {
        let h = headers(&[(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )]);
        assert_eq!(inbound_trace_id(&h), "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn parses_b3_single_and_multi() {
        let single = headers(&[("b3", "80f198ee56343ba864fe8b2a57d3eff7-e457b5a2e4d86bd1-1")]);
        assert_eq!(
            inbound_trace_id(&single),
            "80f198ee56343ba864fe8b2a57d3eff7"
        );
        let multi = headers(&[("x-b3-traceid", "A3CE929D0E0E4736A3CE929D0E0E4736")]);
        assert_eq!(inbound_trace_id(&multi), "a3ce929d0e0e4736a3ce929d0e0e4736");
    }

    #[test]
    fn rejects_malformed_trace_ids() {
        assert_eq!(inbound_trace_id(&headers(&[])), "");
        // wrong length / non-hex are ignored
        assert_eq!(
            inbound_trace_id(&headers(&[("traceparent", "00-xyz-span-01")])),
            ""
        );
        assert_eq!(inbound_trace_id(&headers(&[("b3", "nothex-span")])), "");
    }

    #[test]
    fn response_level_tracks_status_class() {
        use axum::http::StatusCode;
        use tracing::Level;

        // success is quiet (debug), so it stays hidden at the default info filter
        assert_eq!(level_for_status(StatusCode::OK).0, Level::DEBUG);
        assert_eq!(level_for_status(StatusCode::NO_CONTENT).0, Level::DEBUG);
        // client errors (bad key, unknown model) surface as warnings — the ROL-230
        // case that was invisible before
        assert_eq!(level_for_status(StatusCode::NOT_FOUND).0, Level::WARN);
        assert_eq!(level_for_status(StatusCode::UNAUTHORIZED).0, Level::WARN);
        assert_eq!(
            level_for_status(StatusCode::TOO_MANY_REQUESTS).0,
            Level::WARN
        );
        // server / upstream errors are loud
        assert_eq!(level_for_status(StatusCode::BAD_GATEWAY).0, Level::ERROR);
        assert_eq!(
            level_for_status(StatusCode::INTERNAL_SERVER_ERROR).0,
            Level::ERROR
        );
    }

    #[test]
    fn traceparent_wins_over_b3() {
        let h = headers(&[
            (
                "traceparent",
                "00-11111111111111111111111111111111-2222222222222222-01",
            ),
            ("b3", "33333333333333333333333333333333-4444444444444444-1"),
        ]);
        assert_eq!(inbound_trace_id(&h), "11111111111111111111111111111111");
    }

    #[test]
    fn outbound_headers_adds_only_allowlisted_client_headers() {
        let h = headers(&[
            ("traceparent", "00-1111-2222-01"),
            ("x-tenant-id", "acme"),
            ("x-secret", "nope"),
            ("x-blank", ""),
        ]);
        let forwarded = vec!["x-tenant-id".to_string(), "x-blank".to_string()];
        let out = outbound_headers(&h, &forwarded);

        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out.contains(&("traceparent".to_string(), "00-1111-2222-01".to_string())));
        assert!(out.contains(&("x-tenant-id".to_string(), "acme".to_string())));
    }

    #[test]
    fn outbound_headers_never_lets_the_allowlist_duplicate_trace_context() {
        // listing a trace header must not emit it twice: the second copy would
        // be a caller-controlled value racing the context the gateway resolved
        let h = headers(&[("traceparent", "00-1111-2222-01")]);
        let out = outbound_headers(&h, &["traceparent".to_string()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, "00-1111-2222-01");
    }

    #[test]
    fn without_a_pipeline_outbound_headers_still_copies_the_caller_verbatim() {
        // #805 changed where trace context comes from, but only when an OTLP
        // pipeline is installed. with none — the default — the caller's headers
        // are copied exactly as before, so an untraced deployment sees no change
        assert!(!rolter_core::telemetry::is_active());
        let h = headers(&[
            (
                "traceparent",
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            ),
            ("tracestate", "rojo=00f067aa0ba902b7"),
        ]);
        let out = outbound_headers(&h, &[]);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&(
            "traceparent".to_string(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string()
        )));
    }

    #[test]
    fn the_request_span_is_disabled_without_a_pipeline_and_real_with_one() {
        use tower_http::trace::MakeSpan;

        // the trap this pins: `DefaultMakeSpan` builds the request span at DEBUG,
        // which the default `info` filter disables — and a disabled span cannot
        // be parented or exported, so every stage span became a root carrying a
        // freshly-minted trace id. with no pipeline that stock behaviour is what
        // we want (it costs nothing); with one, the span must be real
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .body(())
            .unwrap();

        assert!(!rolter_core::telemetry::is_active());
        let span = GatewayMakeSpan.make_span(&req);
        assert_eq!(
            span.metadata().map(|m| *m.level()),
            Some(tracing::Level::DEBUG),
            "without a pipeline the request span stays the cheap DEBUG one"
        );
    }

    #[test]
    fn request_trace_id_falls_back_to_the_inbound_header() {
        // with no pipeline there is no span context to read, so the log keeps
        // taking the id off the wire exactly as it did before
        let h = headers(&[(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        )]);
        assert_eq!(request_trace_id(&h), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(request_trace_id(&headers(&[])), "");
    }

    #[test]
    fn outbound_headers_with_an_empty_allowlist_is_just_trace_context() {
        let h = headers(&[("traceparent", "00-1111-2222-01"), ("x-tenant-id", "acme")]);
        assert_eq!(outbound_headers(&h, &[]).len(), 1);
    }

    #[test]
    fn outbound_trace_headers_extracts_known_headers() {
        let h = headers(&[
            ("traceparent", "00-1111-2222-01"),
            ("tracestate", "rojo=00f067aa0ba902b7"),
            ("b3", "80f198ee56343ba864fe8b2a57d3eff7-e457b5a2e4d86bd1-1"),
            ("unknown-header", "some-value"),
            ("x-b3-traceid", ""),
        ]);
        let outbound = outbound_trace_headers(&h);
        assert_eq!(outbound.len(), 3);

        let outbound_map: std::collections::HashMap<&str, String> = outbound.into_iter().collect();
        assert_eq!(outbound_map.get("traceparent").unwrap(), "00-1111-2222-01");
        assert_eq!(
            outbound_map.get("tracestate").unwrap(),
            "rojo=00f067aa0ba902b7"
        );
        assert_eq!(
            outbound_map.get("b3").unwrap(),
            "80f198ee56343ba864fe8b2a57d3eff7-e457b5a2e4d86bd1-1"
        );
    }
}
