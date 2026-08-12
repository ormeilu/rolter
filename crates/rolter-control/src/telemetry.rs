//! Per-request spans and latency histograms for the control-plane API (#845).
//!
//! One middleware rather than an attribute on each of ~90 handlers. That is not
//! only less code: a handler-by-handler approach silently stops covering the
//! next endpoint someone adds, and the CRUD surface grows with every feature.
//! Here a new route is instrumented the moment it is mounted.
//!
//! Cardinality is bounded by construction. The label is the *matched* path
//! template (`/api/v1/providers/{id}`), never the concrete path, so a thousand
//! providers are one series and not a thousand — which is exactly what #845
//! rules out.

use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::ControlState;

/// Label used when a request matched no route (a 404). Without this the label
/// would be the caller-supplied path, which is attacker-controlled and
/// unbounded — one scan would create a series per probed URL.
const UNMATCHED: &str = "unmatched";

pub(crate) async fn record_request(
    State(state): State<ControlState>,
    request: Request,
    next: Next,
) -> Response {
    use tracing::Instrument as _;

    // taken before `next.run` consumes the request; the extension is put there
    // by the router, so it is present for anything that matched a route
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string())
        .unwrap_or_else(|| UNMATCHED.to_string());
    let method = request.method().clone();

    let span = rolter_core::stage_span!(
        "control.request",
        http.route = %route,
        http.request.method = %method,
        http.response.status_code = tracing::field::Empty,
    );

    let started = Instant::now();
    let response = next.run(request).instrument(span.clone()).await;
    let status = response.status();
    span.record("http.response.status_code", status.as_u16());

    state.metrics.record_crud(
        &route,
        method.as_str(),
        status.as_u16(),
        started.elapsed().as_millis() as u64,
    );
    response
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::extract::Path;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Router;

    use super::*;

    /// Capture the labels the middleware would record, so the cardinality
    /// claim is asserted rather than asserted-in-a-comment. Mirrors what
    /// `ControlHistograms::record_crud` receives.
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<(String, String, u16)>>,
        calls: AtomicUsize,
    }

    async fn serve(app: Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        addr
    }

    /// The same label extraction the middleware does, over a recorder instead
    /// of a histogram. `record_request` itself needs a `ControlState`, which
    /// pulls in a store and a pool; the part under test is which *labels* come
    /// out, so this exercises that against a real axum router.
    async fn probe(recorder: Arc<Recorder>, request: Request, next: Next) -> Response {
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map(|matched| matched.as_str().to_string())
            .unwrap_or_else(|| UNMATCHED.to_string());
        let method = request.method().as_str().to_string();
        let response = next.run(request).await;
        recorder.calls.fetch_add(1, Ordering::Relaxed);
        recorder
            .seen
            .lock()
            .unwrap()
            .push((route, method, response.status().as_u16()));
        response
    }

    async fn app(recorder: Arc<Recorder>) -> std::net::SocketAddr {
        let router = Router::new()
            .route(
                "/api/v1/providers/{id}",
                get(|Path(id): Path<String>| async move { id }),
            )
            .route("/api/v1/boom", get(|| async { StatusCode::BAD_REQUEST }))
            .layer(axum::middleware::from_fn(move |req, next| {
                let recorder = recorder.clone();
                async move { probe(recorder, req, next).await }
            }));
        serve(router).await
    }

    /// The whole cardinality argument rests on this: two requests to the same
    /// route with different ids must produce the *same* label. If the concrete
    /// path leaked into the label, a thousand providers would be a thousand
    /// series — the unbounded per-entity-id case #845 rules out.
    #[tokio::test]
    async fn the_route_label_is_the_template_not_the_concrete_path() {
        let recorder = Arc::new(Recorder::default());
        let addr = app(recorder.clone()).await;
        let client = reqwest::Client::new();

        for id in ["abc", "def", "0195f0e2-1f4c-7c3a-9a1b-2f8f0d6c4e51"] {
            let response = client
                .get(format!("http://{addr}/api/v1/providers/{id}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
        }

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        for (route, method, status) in seen.iter() {
            assert_eq!(route, "/api/v1/providers/{id}", "concrete path leaked");
            assert_eq!(method, "GET");
            assert_eq!(*status, 200);
        }
    }

    /// An unmatched path is caller-controlled and unbounded — a scan would
    /// otherwise create one series per probed URL.
    #[tokio::test]
    async fn an_unmatched_path_is_labelled_with_a_constant() {
        let recorder = Arc::new(Recorder::default());
        let addr = app(recorder.clone()).await;
        let client = reqwest::Client::new();

        for path in ["/nope", "/wp-admin", "/api/v1/../../etc/passwd"] {
            let _ = client.get(format!("http://{addr}{path}")).send().await;
        }

        let seen = recorder.seen.lock().unwrap();
        for (route, _, _) in seen.iter() {
            assert_eq!(route, UNMATCHED, "an unmatched path became a label");
        }
    }

    /// Observing must not change what the API returns.
    #[tokio::test]
    async fn responses_pass_through_unchanged() {
        let recorder = Arc::new(Recorder::default());
        let addr = app(recorder.clone()).await;
        let client = reqwest::Client::new();

        let ok = client
            .get(format!("http://{addr}/api/v1/providers/abc"))
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.text().await.unwrap(), "abc");

        let bad = client
            .get(format!("http://{addr}/api/v1/boom"))
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 400);
        assert_eq!(recorder.calls.load(Ordering::Relaxed), 2);
    }

    /// The recorders are inert without an OTLP endpoint: every deployment that
    /// exports nothing must pay nothing and panic on nothing.
    #[test]
    fn the_histograms_are_inert_without_otlp() {
        let histograms = rolter_core::telemetry::ControlHistograms::default();
        assert!(!histograms.is_active());
        histograms.record_crud("/api/v1/providers/{id}", "GET", 200, 3);
        histograms.record_snapshot("ok", 12, 4096);
        histograms.record_snapshot("not_modified", 1, 0);
    }
}
