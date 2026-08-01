//! Global feature-flag API for safely hot-reloadable runtime subsystems.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use rolter_core::Error;
use rolter_store::postgres::models::FeatureFlags;
use rolter_store::postgres::repo::{AuditLogRepo, FeatureFlagsRepo};

use crate::crud::{pool, publish_config_change, ApiError, ApiResult, SafeJson};
use crate::rbac::{authorize_superadmin, Principal};
use crate::rbac_matrix::superadmin_cap;
use crate::ControlState;

pub(crate) fn router() -> Router<ControlState> {
    Router::new().route(
        "/api/v1/feature-flags",
        get(get_feature_flags).put(update_feature_flags),
    )
}

/// A flag whose subsystem is not deployable in this deployment. The screen
/// renders these as unavailable rather than as a switch that silently does
/// nothing when flipped (#535).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct UnavailableFlag {
    pub(crate) flag: &'static str,
    pub(crate) reason: &'static str,
}

/// Stored flags plus what this deployment can actually run.
#[derive(Debug, Serialize)]
struct FeatureFlagsView {
    #[serde(flatten)]
    flags: FeatureFlags,
    unavailable: Vec<UnavailableFlag>,
}

/// Which flags have no working subsystem behind them here. Both checks read
/// deployment facts, not the flags themselves, so a flag left enabled from an
/// earlier deployment still reports as unavailable.
pub(crate) fn unavailable_flags(
    redis_configured: bool,
    cache_aware_providers: bool,
) -> Vec<UnavailableFlag> {
    let mut out = Vec::new();
    if !redis_configured {
        out.push(UnavailableFlag {
            flag: "response_cache",
            reason: "no redis url is configured; cache entries are shared through redis",
        });
    }
    if !cache_aware_providers {
        out.push(UnavailableFlag {
            flag: "cache_aware_routing",
            reason: "no provider publishes kv-cache events or lmcache metrics",
        });
    }
    out
}

/// Deployment facts the availability check needs, read from the effective
/// config rather than the flag row.
async fn deployment_capabilities(state: &ControlState) -> (bool, bool) {
    let cache_aware_providers = match state.store.load().await {
        Ok(config) => config
            .providers
            .iter()
            .any(|p| p.kv_events.is_some() || p.lmcache.is_some()),
        Err(err) => {
            tracing::warn!(error = %err, "failed to load config for flag availability");
            false
        }
    };
    (state.redis.is_some(), cache_aware_providers)
}

async fn get_feature_flags(
    principal: Principal,
    State(state): State<ControlState>,
) -> ApiResult<Json<FeatureFlagsView>> {
    authorize_superadmin(&principal, superadmin_cap!("feature_flags", Read))?;
    let flags = FeatureFlagsRepo(pool(&state)).get().await?;
    let (redis, cache_aware) = deployment_capabilities(&state).await;
    Ok(Json(FeatureFlagsView {
        flags,
        unavailable: unavailable_flags(redis, cache_aware),
    }))
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

/// Reject turning on a flag whose subsystem cannot run here — the switch would
/// persist and publish a policy the gateway silently ignores.
fn reject_unavailable(body: &UpdateFeatureFlags, unavailable: &[UnavailableFlag]) -> ApiResult<()> {
    for entry in unavailable {
        let requested = match entry.flag {
            "response_cache" => body.response_cache,
            "cache_aware_routing" => body.cache_aware_routing,
            _ => false,
        };
        if requested {
            return Err(ApiError::Core(Error::Config(format!(
                "{} is unavailable: {}",
                entry.flag, entry.reason
            ))));
        }
    }
    Ok(())
}

async fn update_feature_flags(
    principal: Principal,
    State(state): State<ControlState>,
    SafeJson(body): SafeJson<UpdateFeatureFlags>,
) -> ApiResult<Json<FeatureFlagsView>> {
    authorize_superadmin(&principal, superadmin_cap!("feature_flags", Update))?;
    let (redis, cache_aware) = deployment_capabilities(&state).await;
    let unavailable = unavailable_flags(redis, cache_aware);
    reject_unavailable(&body, &unavailable)?;
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
    Ok(Json(FeatureFlagsView {
        flags: row,
        unavailable,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(response_cache: bool, cache_aware_routing: bool) -> UpdateFeatureFlags {
        UpdateFeatureFlags {
            response_cache,
            cache_aware_routing,
            circuit_breaker: true,
            active_health_checks: true,
            complexity_routing: true,
            guardrails: true,
        }
    }

    #[test]
    fn a_fully_capable_deployment_reports_nothing_unavailable() {
        assert!(unavailable_flags(true, true).is_empty());
        assert!(reject_unavailable(&flags(true, true), &[]).is_ok());
    }

    #[test]
    fn enabling_an_unavailable_flag_is_rejected() {
        let unavailable = unavailable_flags(false, false);
        assert_eq!(unavailable.len(), 2);
        assert!(reject_unavailable(&flags(true, false), &unavailable).is_err());
        assert!(reject_unavailable(&flags(false, true), &unavailable).is_err());
        // leaving them off is still allowed, so the row stays editable
        assert!(reject_unavailable(&flags(false, false), &unavailable).is_ok());
    }
}
