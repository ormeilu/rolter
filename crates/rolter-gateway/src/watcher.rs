//! Reload-free config watcher.
//!
//! Polls the control plane's `GET /internal/snapshot?version=N` on an
//! interval and hot-swaps the gateway's routing [`Snapshot`] via
//! [`AppState::reload`] whenever a newer config version is published. The
//! control plane replies `304 Not Modified` when the gateway is already
//! current, so steady-state polling is cheap.
//!
//! Polling is the reliable baseline transport. When a redis url is
//! configured, a subscriber on [`rolter_core::CONFIG_CHANNEL`] additionally
//! wakes the loop the moment the control plane publishes a version bump, so
//! changes propagate immediately instead of waiting out the poll interval.
//! Redis being down never breaks config propagation — the subscriber
//! reconnects with backoff while interval polling keeps working.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tokio::sync::Notify;

use rolter_core::GatewayConfig;

use crate::state::AppState;

/// A `{"version": N, "config": {...}}` document from the snapshot endpoint.
#[derive(Deserialize)]
struct SnapshotResponse {
    version: u64,
    config: GatewayConfig,
}

/// Spawn the background watcher. `snapshot_url` is the control plane's
/// snapshot endpoint (e.g. `http://control:4001/internal/snapshot`); `period`
/// is the poll interval; `redis_url`, when set, adds an instant pub/sub
/// wake-up on top of the interval. Returns immediately; the tasks run until
/// the process exits.
pub fn spawn(
    state: AppState,
    snapshot_url: String,
    period: Duration,
    redis_url: Option<String>,
    admin_token: Option<String>,
) {
    let wakeup = Arc::new(Notify::new());
    if let Some(url) = redis_url {
        tokio::spawn(subscribe(url, Arc::clone(&wakeup)));
    }
    tokio::spawn(async move {
        // a dedicated short-timeout client so a hung control plane can't wedge
        // the watcher loop
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        run(
            &client,
            &state,
            &snapshot_url,
            admin_token.as_deref(),
            period,
            &wakeup,
        )
        .await;
    });
}

/// The poll loop, factored out so tests can drive a single tick. Polls on
/// the interval and immediately whenever the redis subscriber signals.
async fn run(
    client: &Client,
    state: &AppState,
    snapshot_url: &str,
    admin_token: Option<&str>,
    period: Duration,
    wakeup: &Notify,
) {
    let mut ticker = tokio::time::interval(period);
    // the initial config was applied at startup; treat it as version 0 so the
    // first successful poll always applies the authoritative version
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = wakeup.notified() => {}
        }
        if let Err(err) = poll_once(client, state, snapshot_url, admin_token).await {
            state
                .metrics
                .config_reload_failures_total
                .fetch_add(1, Relaxed);
            tracing::warn!(error = %err, "config snapshot poll failed");
        }
    }
}

/// Subscribe to [`rolter_core::CONFIG_CHANNEL`] and signal `wakeup` on every
/// message. Reconnects with a fixed backoff forever; interval polling covers
/// propagation while redis is unavailable.
async fn subscribe(redis_url: String, wakeup: Arc<Notify>) {
    const BACKOFF: Duration = Duration::from_secs(5);
    loop {
        let session = async {
            let client = redis::Client::open(redis_url.as_str())?;
            let mut pubsub = client.get_async_pubsub().await?;
            pubsub.subscribe(rolter_core::CONFIG_CHANNEL).await?;
            tracing::info!(
                channel = rolter_core::CONFIG_CHANNEL,
                "config pub/sub connected"
            );
            let mut stream = pubsub.on_message();
            while stream.next().await.is_some() {
                wakeup.notify_one();
            }
            Ok::<_, redis::RedisError>(())
        };
        if let Err(err) = session.await {
            tracing::warn!(error = %err, "config pub/sub disconnected; retrying");
        }
        tokio::time::sleep(BACKOFF).await;
    }
}

/// Header names this node identifies itself with on each poll, so the control
/// plane can keep a cluster inventory without a second channel (#543).
const NODE_ID_HEADER: &str = "x-rolter-node-id";
const NODE_ROLE_HEADER: &str = "x-rolter-node-role";
const NODE_BUILD_HEADER: &str = "x-rolter-node-build";
/// Response header carrying the operator-requested state for this node.
const NODE_STATE_HEADER: &str = "x-rolter-node-state";

/// Stable identity for this gateway process. `ROLTER_NODE_ID` when set (the
/// deployment's own name for the replica), otherwise the hostname, otherwise
/// nothing — an unidentified node polls exactly as before and stays out of the
/// inventory rather than churning it with a per-restart id.
fn node_id() -> Option<String> {
    for key in ["ROLTER_NODE_ID", "HOSTNAME"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Fetch the snapshot once and apply it if newer. Returns `Ok(Some(version))`
/// when a reload happened, `Ok(None)` on `304`/no-change, `Err` on transport
/// or decode failure.
async fn poll_once(
    client: &Client,
    state: &AppState,
    snapshot_url: &str,
    admin_token: Option<&str>,
) -> anyhow::Result<Option<u64>> {
    let current = state.metrics.config_version.load(Relaxed);
    let mut request = client
        .get(snapshot_url)
        .query(&[("version", current.to_string())]);
    if let Some(token) = admin_token {
        request = request.bearer_auth(token);
    }
    if let Some(id) = node_id() {
        request = request
            .header(NODE_ID_HEADER, id)
            .header(NODE_ROLE_HEADER, "gateway")
            .header(NODE_BUILD_HEADER, env!("CARGO_PKG_VERSION"));
    }
    let resp = request.send().await?;

    // the control plane answers every poll with the node's requested state, so
    // a drain lands even when the config itself has not changed
    apply_node_state(state, resp.headers());

    if resp.status() == StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !resp.status().is_success() {
        anyhow::bail!("snapshot endpoint returned {}", resp.status());
    }

    let body: SnapshotResponse = resp.json().await?;
    // guard against a stale/racy response older than what we already run
    if body.version <= current && current != 0 {
        return Ok(None);
    }
    // never apply a broken snapshot; keep serving the last good config
    if let Err(problems) = body.config.validate() {
        anyhow::bail!("snapshot v{} failed validation: {problems:?}", body.version);
    }
    state.reload(&body.config, body.version);
    tracing::info!(version = body.version, "applied new config snapshot");
    Ok(Some(body.version))
}

/// Apply the operator-requested node state carried on the snapshot response.
/// A response without the header leaves the current state alone, so an
/// unmanaged or unidentified node never flips itself out of service.
fn apply_node_state(state: &AppState, headers: &reqwest::header::HeaderMap) {
    let Some(requested) = headers.get(NODE_STATE_HEADER).and_then(|v| v.to_str().ok()) else {
        return;
    };
    let draining = requested.eq_ignore_ascii_case("draining");
    let previous = state.draining.swap(draining, Relaxed);
    if previous != draining {
        tracing::info!(draining, "control plane changed this node's serving state");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn applies_newer_version_and_skips_same() {
        // spin a tiny control-plane stub that serves an incrementing version
        use axum::extract::Query;
        use axum::routing::get;
        use axum::{Json, Router};
        use std::collections::HashMap;

        async fn snapshot(Query(q): Query<HashMap<String, String>>) -> axum::response::Response {
            use axum::response::IntoResponse;
            let seen: u64 = q.get("version").and_then(|v| v.parse().ok()).unwrap_or(0);
            // authoritative version is 5; reply 304 once the caller has it
            if seen >= 5 {
                return axum::http::StatusCode::NOT_MODIFIED.into_response();
            }
            Json(serde_json::json!({
                "version": 5,
                "config": {"server": {"host": "0.0.0.0", "port": 4000}}
            }))
            .into_response()
        }

        let app = Router::new().route("/internal/snapshot", get(snapshot));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let state = AppState::new(&GatewayConfig::default());
        let client = Client::new();
        let url = format!("http://{addr}/internal/snapshot");

        // first poll applies version 5
        let applied = poll_once(&client, &state, &url, None).await.unwrap();
        assert_eq!(applied, Some(5));
        assert_eq!(state.metrics.config_version.load(Relaxed), 5);
        assert_eq!(state.metrics.config_reloads_total.load(Relaxed), 1);

        // second poll is a no-op (control replies 304)
        let applied = poll_once(&client, &state, &url, None).await.unwrap();
        assert_eq!(applied, None);
        assert_eq!(state.metrics.config_reloads_total.load(Relaxed), 1);
    }

    #[tokio::test]
    async fn drain_state_from_the_snapshot_response_flips_readiness() {
        use reqwest::header::{HeaderMap, HeaderValue};

        let state = AppState::new(&GatewayConfig::default());
        assert!(!state.draining.load(Relaxed));

        // a response with no header leaves the node serving
        apply_node_state(&state, &HeaderMap::new());
        assert!(!state.draining.load(Relaxed));

        let mut draining = HeaderMap::new();
        draining.insert(NODE_STATE_HEADER, HeaderValue::from_static("draining"));
        apply_node_state(&state, &draining);
        assert!(state.draining.load(Relaxed));

        // an unchanged poll must not silently return the node to service
        apply_node_state(&state, &HeaderMap::new());
        assert!(state.draining.load(Relaxed));

        let mut active = HeaderMap::new();
        active.insert(NODE_STATE_HEADER, HeaderValue::from_static("active"));
        apply_node_state(&state, &active);
        assert!(!state.draining.load(Relaxed));
    }

    #[tokio::test]
    async fn rejects_invalid_snapshot_and_keeps_old_config() {
        use axum::routing::get;
        use axum::{Json, Router};

        // route targets a provider that doesn't exist -> must be rejected
        async fn snapshot() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "version": 9,
                "config": {
                    "routes": [{"model": "broken", "targets": [{"provider": "ghost"}]}]
                }
            }))
        }

        let app = Router::new().route("/internal/snapshot", get(snapshot));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let state = AppState::new(&GatewayConfig::default());
        let client = Client::new();
        let url = format!("http://{addr}/internal/snapshot");

        let res = poll_once(&client, &state, &url, None).await;
        assert!(res.is_err(), "invalid snapshot must be an error");
        // old config stays: no reload recorded, version unchanged
        assert_eq!(state.metrics.config_reloads_total.load(Relaxed), 0);
        assert_eq!(state.metrics.config_version.load(Relaxed), 0);
    }

    #[tokio::test]
    async fn counts_failure_on_unreachable_control() {
        let state = AppState::new(&GatewayConfig::default());
        let client = Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        // nothing is listening here
        let err = poll_once(
            &client,
            &state,
            "http://127.0.0.1:1/internal/snapshot",
            None,
        )
        .await;
        assert!(err.is_err());
    }
}
