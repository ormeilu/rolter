//! Cluster inventory (#543).
//!
//! Nodes are not enrolled through a separate API: every gateway already polls
//! `/internal/snapshot` on a timer, so a poll that carries the node identity
//! headers *is* the heartbeat. That keeps single-node deployments zero-config —
//! a node that sends no headers simply never appears in the inventory — and
//! keeps config propagation the one source of truth for convergence.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use rolter_store::postgres::models::ClusterNode;
use rolter_store::postgres::repo::{AuditLogRepo, ClusterNodeRepo};

use crate::crud::{pool, ApiResult};
use crate::rbac::{require_superadmin, Principal};
use crate::ControlState;

/// Header names a node uses to identify itself on its snapshot poll.
pub(crate) const NODE_ID_HEADER: &str = "x-rolter-node-id";
pub(crate) const NODE_ROLE_HEADER: &str = "x-rolter-node-role";
pub(crate) const NODE_BUILD_HEADER: &str = "x-rolter-node-build";

/// A node is considered live if it polled within this window. The default
/// snapshot poll period is measured in seconds, so a node that misses several
/// polls in a row is stale rather than merely slow.
const LIVENESS_WINDOW_SECS: i64 = 60;

pub(crate) fn router() -> Router<ControlState> {
    Router::new()
        .route("/api/v1/cluster/nodes", get(list_nodes))
        .route(
            "/api/v1/cluster/nodes/{id}",
            axum::routing::delete(forget_node),
        )
}

/// Whether a node is still polling, and whether it runs the current config.
#[derive(Debug, Serialize)]
struct ClusterNodeView {
    #[serde(flatten)]
    node: ClusterNode,
    /// true when the node polled inside the liveness window
    live: bool,
    /// true when the node reported the control plane's current config version
    converged: bool,
}

fn view(node: ClusterNode, now: DateTime<Utc>, current_version: i64) -> ClusterNodeView {
    let live =
        now.signed_duration_since(node.last_seen_at) <= Duration::seconds(LIVENESS_WINDOW_SECS);
    let converged = node.config_version >= current_version;
    ClusterNodeView {
        node,
        live,
        converged,
    }
}

/// Sanitize a node-supplied header value. Identity comes from a node holding
/// the internal token, but the value still reaches the database and the
/// dashboard, so it is length-bounded and stripped of control characters.
fn header_value(headers: &HeaderMap, name: &str, max_len: usize) -> Option<String> {
    let raw = headers.get(name)?.to_str().ok()?.trim();
    if raw.is_empty() || raw.len() > max_len || raw.chars().any(char::is_control) {
        return None;
    }
    Some(raw.to_string())
}

/// Record a snapshot poll as a node sighting. Silent when the caller sends no
/// identity (an anonymous poller stays out of the inventory) and best-effort
/// otherwise: a bookkeeping failure must never fail config propagation.
pub(crate) async fn record_heartbeat(
    state: &ControlState,
    headers: &HeaderMap,
    reported_version: Option<i64>,
) {
    let Some(pool) = state.pool.as_ref() else {
        return;
    };
    let Some(id) = header_value(headers, NODE_ID_HEADER, 128) else {
        return;
    };
    let role = match header_value(headers, NODE_ROLE_HEADER, 16).as_deref() {
        Some("control") => "control",
        // an unrecognized role is recorded as a gateway rather than dropped;
        // the inventory is more useful with the node than without it
        _ => "gateway",
    };
    let build = header_value(headers, NODE_BUILD_HEADER, 64).unwrap_or_default();
    if let Err(err) = ClusterNodeRepo(pool)
        .heartbeat(&id, role, &build, reported_version.unwrap_or_default())
        .await
    {
        tracing::warn!(error = %err, node = %id, "failed to record cluster node heartbeat");
    }
}

async fn list_nodes(
    principal: Principal,
    State(state): State<ControlState>,
) -> ApiResult<Json<Vec<ClusterNodeView>>> {
    require_superadmin(&principal)?;
    let nodes = ClusterNodeRepo(pool(&state)).list().await?;
    let current_version = state.store.current_version().await.unwrap_or_default();
    let now = Utc::now();
    Ok(Json(
        nodes
            .into_iter()
            .map(|node| view(node, now, current_version))
            .collect(),
    ))
}

/// Drop a decommissioned node from the inventory. A node that is still running
/// reappears on its next poll, so this cannot be used to silence a live node.
async fn forget_node(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_superadmin(&principal)?;
    ClusterNodeRepo(pool(&state)).delete(&id).await?;
    let actor = match &principal {
        Principal::User(user) => Some(user.id),
        Principal::Superadmin => None,
    };
    if let Err(err) = AuditLogRepo(pool(&state))
        .create(
            None,
            actor,
            "cluster_node.forget",
            Some("cluster_node"),
            None,
            Some(serde_json::json!({ "node_id": id })),
        )
        .await
    {
        tracing::warn!(error = %err, "failed to write cluster node audit log");
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn node(last_seen_at: DateTime<Utc>, config_version: i64) -> ClusterNode {
        ClusterNode {
            id: "gw-1".to_string(),
            role: "gateway".to_string(),
            build_version: "0.0.10".to_string(),
            config_version,
            first_seen_at: last_seen_at,
            last_seen_at,
        }
    }

    #[test]
    fn liveness_and_convergence_are_reported_separately() {
        let now = Utc::now();
        // a node polling now on the current version
        let fresh = view(node(now, 7), now, 7);
        assert!(fresh.live && fresh.converged);
        // still polling, but has not applied the newest snapshot yet
        let lagging = view(node(now, 6), now, 7);
        assert!(lagging.live && !lagging.converged);
        // stopped polling while on the current version: dead, not converged-safe
        let stale = view(node(now - Duration::seconds(120), 7), now, 7);
        assert!(!stale.live && stale.converged);
    }

    #[test]
    fn node_headers_are_bounded_and_control_free() {
        let mut headers = HeaderMap::new();
        headers.insert(NODE_ID_HEADER, HeaderValue::from_static("gw-1"));
        assert_eq!(
            header_value(&headers, NODE_ID_HEADER, 128).as_deref(),
            Some("gw-1")
        );
        // over-long values are dropped rather than truncated into a new identity
        assert_eq!(header_value(&headers, NODE_ID_HEADER, 2), None);
        // and a missing header is simply absent
        assert_eq!(header_value(&headers, NODE_ROLE_HEADER, 16), None);
    }
}
