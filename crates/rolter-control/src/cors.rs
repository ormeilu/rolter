//! Cross-origin policy for the control-plane API (#813).
//!
//! `security_settings.allowed_origins` / `allowed_headers` were stored,
//! validated and served for a long time with no consumer anywhere in the
//! workspace — an operator could save an origin, receive a `200`, and have
//! configured nothing but a database row. This module is that consumer.
//!
//! Only the control plane gets a cross-origin policy, because it is the only
//! origin a browser talks to: the dashboard is served by the control plane, and
//! its Playground reaches the data plane through the `/gw/*` reverse proxy
//! rather than by calling the gateway directly. A gateway CORS layer would
//! govern requests that browsers do not make, and would need the settings in
//! the snapshot to do it.
//!
//! The policy is read from an [`arc_swap::ArcSwap`] on every request, so an edit
//! through `PUT /api/v1/security-settings` takes effect immediately rather than
//! at the next restart. With no origins configured — the default, and the
//! same-origin deployment everyone runs — the middleware adds no headers and
//! behaves exactly as before this existed.

use axum::extract::{Request, State};
use axum::http::header::{HeaderName, HeaderValue, VARY};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::ControlState;

/// Request headers a cross-origin caller may always send, unioned with whatever
/// the operator configured.
///
/// `authorization` and `content-type` are what the dashboard cannot work
/// without, so leaving them to configuration would mean a split-origin
/// deployment breaks until someone remembers them. `traceparent` / `tracestate`
/// are here for #805: without them the dashboard's spans cannot join the
/// gateway's on a split-origin deployment, and browser trace propagation
/// silently does not work. `x-request-id` is the correlation id the API echoes.
#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
const BASELINE_HEADERS: &[&str] = &[
    "authorization",
    "content-type",
    "traceparent",
    "tracestate",
    "x-request-id",
];

/// Response headers a cross-origin caller may read.
///
/// `x-request-id` is stamped on every response and is the id an operator quotes
/// in a bug report; without exposing it the browser cannot see it at all.
const EXPOSED_HEADERS: &str = "x-request-id";

/// Methods the API serves. Fixed rather than derived from the router, since a
/// preflight asks about one method and the answer is the same for every route.
const ALLOWED_METHODS: &str = "GET, POST, PUT, PATCH, DELETE, OPTIONS";

/// How long a browser may cache a preflight, in seconds.
const MAX_AGE: &str = "600";

/// The live cross-origin policy, rebuilt whenever security settings change.
///
/// Empty `origins` means "no cross-origin policy configured", which is the
/// default and is checked before anything else on the request path.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CorsPolicy {
    origins: Vec<String>,
    /// pre-joined for the `Access-Control-Allow-Headers` response, so a
    /// preflight does no string building
    allow_headers: String,
}

impl CorsPolicy {
    /// Build a policy from the stored settings.
    ///
    /// Origins are normalized by trimming a trailing slash, because the browser
    /// sends an `Origin` without one and an operator may well save one with.
    /// Headers are lowercased and deduplicated against the baseline set.
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    pub(crate) fn new(origins: &[String], headers: &[String]) -> Self {
        let origins: Vec<String> = origins
            .iter()
            .map(|o| o.trim().trim_end_matches('/').to_ascii_lowercase())
            .filter(|o| !o.is_empty())
            .collect();

        let mut allow: Vec<String> = BASELINE_HEADERS.iter().map(|h| (*h).to_string()).collect();
        for header in headers {
            let header = header.trim().to_ascii_lowercase();
            // an invalid name cannot appear in a real preflight, so echoing it
            // would only ever widen the advertised set with noise
            if header.is_empty() || HeaderName::from_bytes(header.as_bytes()).is_err() {
                continue;
            }
            if !allow.contains(&header) {
                allow.push(header);
            }
        }

        Self {
            origins,
            allow_headers: allow.join(", "),
        }
    }

    /// Whether any cross-origin access is configured at all.
    fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    /// The exact origin to echo back, when `origin` is on the allow-list.
    ///
    /// Matching is exact against the configured list — never a wildcard, never a
    /// suffix match. `validate_origin` already rejects `*`, and reflecting an
    /// arbitrary origin would make the allow-list decorative.
    fn allows(&self, origin: &str) -> bool {
        let candidate = origin.trim().trim_end_matches('/').to_ascii_lowercase();
        self.origins.contains(&candidate)
    }
}

/// Apply the live cross-origin policy to one request.
///
/// A request with no `Origin` header, or a deployment with no origins
/// configured, passes through completely untouched.
pub(crate) async fn apply_cors(
    State(state): State<ControlState>,
    request: Request,
    next: Next,
) -> Response {
    let policy = state.cors.load();
    if policy.is_empty() {
        return next.run(request).await;
    }

    let origin = request
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let Some(origin) = origin else {
        return next.run(request).await;
    };

    let allowed = policy
        .allows(&origin)
        .then(|| HeaderValue::from_str(&origin));
    let is_preflight = request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key("access-control-request-method");

    // a preflight is answered here and never reaches the router: it carries no
    // credentials, so routing it would only produce a 401 or 405 the browser
    // reads as a denied preflight
    if is_preflight {
        let mut response = StatusCode::NO_CONTENT.into_response();
        // Vary goes on every answer, allowed or not, so a shared cache cannot
        // serve one origin's answer to another
        vary_on_origin(&mut response);
        if let Some(Ok(value)) = allowed {
            let headers = response.headers_mut();
            headers.insert("access-control-allow-origin", value);
            headers.insert(
                "access-control-allow-methods",
                HeaderValue::from_static(ALLOWED_METHODS),
            );
            if let Ok(value) = HeaderValue::from_str(&policy.allow_headers) {
                headers.insert("access-control-allow-headers", value);
            }
            headers.insert("access-control-max-age", HeaderValue::from_static(MAX_AGE));
        }
        return response;
    }

    let mut response = next.run(request).await;
    vary_on_origin(&mut response);
    if let Some(Ok(value)) = allowed {
        let headers = response.headers_mut();
        headers.insert("access-control-allow-origin", value);
        headers.insert(
            "access-control-expose-headers",
            HeaderValue::from_static(EXPOSED_HEADERS),
        );
    }
    response
}

/// Append `Origin` to `Vary` without dropping whatever is already there.
fn vary_on_origin(response: &mut Response) {
    let headers = response.headers_mut();
    let merged = match headers.get(VARY).and_then(|v| v.to_str().ok()) {
        Some(existing) if existing.to_ascii_lowercase().contains("origin") => return,
        Some(existing) => format!("{existing}, origin"),
        None => "origin".to_string(),
    };
    if let Ok(value) = HeaderValue::from_str(&merged) {
        headers.insert(VARY, value);
    }
}

/// Load the stored policy into `state.cors`.
///
/// Called once at startup and again after every security-settings write, so the
/// live policy and the database never disagree for longer than one request.
#[cfg(feature = "postgres")]
pub(crate) async fn refresh(state: &ControlState) {
    use std::sync::Arc;

    use rolter_store::postgres::repo::SecuritySettingsRepo;

    let Some(pool) = state.pool.as_ref() else {
        return;
    };
    match SecuritySettingsRepo(pool).get().await {
        Ok(settings) => {
            state.cors.store(Arc::new(CorsPolicy::new(
                &settings.allowed_origins,
                &settings.allowed_headers,
            )));
        }
        // a failed read must not take the control plane down; the previous
        // policy stays in force and the next write refreshes it again
        Err(err) => tracing::warn!(error = %err, "could not refresh the cross-origin policy"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> CorsPolicy {
        CorsPolicy::new(
            &["https://dash.example.com/".to_string()],
            &["x-tenant".to_string()],
        )
    }

    #[test]
    fn no_origins_configured_is_no_policy() {
        assert!(CorsPolicy::default().is_empty());
        assert!(CorsPolicy::new(&[], &["x-tenant".to_string()]).is_empty());
    }

    #[test]
    fn matching_is_exact_and_slash_insensitive() {
        let p = policy();
        assert!(p.allows("https://dash.example.com"));
        assert!(p.allows("https://dash.example.com/"));
        assert!(p.allows("HTTPS://DASH.EXAMPLE.COM"));
    }

    #[test]
    fn a_lookalike_origin_is_not_allowed() {
        // the failure mode a suffix or prefix match would introduce: an attacker
        // registering evil-dash.example.com or dash.example.com.evil.test must
        // not inherit the allow-list
        let p = policy();
        assert!(!p.allows("https://evil-dash.example.com"));
        assert!(!p.allows("https://dash.example.com.evil.test"));
        assert!(!p.allows("http://dash.example.com"));
        assert!(!p.allows("*"));
        assert!(!p.allows(""));
    }

    #[test]
    fn trace_headers_are_always_allowed() {
        // #805: without these the dashboard's spans cannot join the gateway's
        // on a split-origin deployment, and it fails silently
        let allow = policy().allow_headers;
        assert!(allow.contains("traceparent"));
        assert!(allow.contains("tracestate"));
        assert!(allow.contains("authorization"));
        assert!(allow.contains("content-type"));
        // the configured one is unioned in, not replaced
        assert!(allow.contains("x-tenant"));
    }

    #[test]
    fn configured_headers_are_deduplicated_and_normalized() {
        let p = CorsPolicy::new(
            &["https://a.test".to_string()],
            &[
                "Authorization".to_string(),
                "  X-Tenant  ".to_string(),
                String::new(),
                "not a header".to_string(),
            ],
        );
        assert_eq!(p.allow_headers.matches("authorization").count(), 1);
        assert!(p.allow_headers.contains("x-tenant"));
        assert!(!p.allow_headers.contains("not a header"));
    }
}
