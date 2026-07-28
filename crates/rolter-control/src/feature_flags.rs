//! Global feature-flag API for safely hot-reloadable runtime subsystems.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use rolter_store::postgres::models::FeatureFlags;
use rolter_store::postgres::repo::{AuditLogRepo, FeatureFlagsRepo};

use crate::crud::{pool, publish_config_change, ApiResult, SafeJson};
use crate::rbac::{require_superadmin, Principal};
use crate::ControlState;

pub(crate) fn router() -> Router<ControlState> {
    Router::new().route(
        "/api/v1/feature-flags",
        get(get_feature_flags).put(update_feature_flags),
    )
}

async fn get_feature_flags(
    principal: Principal,
    State(state): State<ControlState>,
) -> ApiResult<Json<FeatureFlags>> {
    require_superadmin(&principal)?;
    Ok(Json(FeatureFlagsRepo(pool(&state)).get().await?))
}

#[derive(Deserialize)]
struct UpdateFeatureFlags {
    response_cache: bool,
    cache_aware_routing: bool,
    circuit_breaker: bool,
    active_health_checks: bool,
    complexity_routing: bool,
    guardrails: bool,
}

async fn update_feature_flags(
    principal: Principal,
    State(state): State<ControlState>,
    SafeJson(body): SafeJson<UpdateFeatureFlags>,
) -> ApiResult<Json<FeatureFlags>> {
    require_superadmin(&principal)?;
    let row = FeatureFlagsRepo(pool(&state))
        .update(
            body.response_cache,
            body.cache_aware_routing,
            body.circuit_breaker,
            body.active_health_checks,
            body.complexity_routing,
            body.guardrails,
        )
        .await?;
    publish_config_change(&state).await?;
    let actor = match &principal {
        Principal::User(user) => Some(user.id),
        Principal::Superadmin => None,
    };
    if let Err(err) = AuditLogRepo(pool(&state))
        .create(
            None,
            actor,
            "feature_flags.update",
            Some("feature_flags"),
            None,
            Some(serde_json::json!({
                "response_cache": row.response_cache,
                "cache_aware_routing": row.cache_aware_routing,
                "circuit_breaker": row.circuit_breaker,
                "active_health_checks": row.active_health_checks,
                "complexity_routing": row.complexity_routing,
                "guardrails": row.guardrails,
            })),
        )
        .await
    {
        tracing::warn!(error = %err, "failed to write feature flags audit log");
    }
    Ok(Json(row))
}
