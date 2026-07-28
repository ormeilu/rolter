//! Global logging-policy API for request-log sampling and payload capture.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use rolter_core::Error;
use rolter_store::postgres::models::LoggingSettings;
use rolter_store::postgres::repo::{AuditLogRepo, LoggingSettingsRepo};

use crate::crud::{pool, publish_config_change, ApiError, ApiResult, SafeJson};
use crate::rbac::{require_superadmin, Principal};
use crate::ControlState;

pub(crate) fn router() -> Router<ControlState> {
    Router::new().route(
        "/api/v1/logging-settings",
        get(get_logging_settings).put(update_logging_settings),
    )
}

async fn get_logging_settings(
    principal: Principal,
    State(state): State<ControlState>,
) -> ApiResult<Json<LoggingSettings>> {
    require_superadmin(&principal)?;
    Ok(Json(LoggingSettingsRepo(pool(&state)).get().await?))
}

#[derive(Deserialize)]
struct UpdateLoggingSettings {
    sample_rate: f64,
    payload_capture_enabled: bool,
    payload_capture_max_bytes: i32,
    #[serde(default)]
    payload_capture_redact_fields: Vec<String>,
    #[serde(default)]
    payload_capture_models: Vec<String>,
    #[serde(default)]
    payload_capture_virtual_key_ids: Vec<String>,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::Core(Error::Config(message.into()))
}

fn validate_settings(body: &UpdateLoggingSettings) -> ApiResult<()> {
    if !body.sample_rate.is_finite() || !(0.0..=1.0).contains(&body.sample_rate) {
        return Err(invalid("sample_rate must be in [0, 1]"));
    }
    if !(0..=1_048_576).contains(&body.payload_capture_max_bytes) {
        return Err(invalid(
            "payload_capture_max_bytes must be between 0 and 1048576",
        ));
    }
    if body.payload_capture_redact_fields.iter().any(|field| {
        field.trim().is_empty() || field.len() > 128 || field.chars().any(char::is_control)
    }) {
        return Err(invalid(
            "payload_capture_redact_fields must contain 1-128 visible-character keys",
        ));
    }
    if body.payload_capture_models.iter().any(|model| {
        model.trim().is_empty() || model.len() > 255 || model.chars().any(char::is_control)
    }) {
        return Err(invalid(
            "payload_capture_models must contain 1-255 visible-character model names",
        ));
    }
    if body
        .payload_capture_virtual_key_ids
        .iter()
        .any(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err(invalid(
            "payload_capture_virtual_key_ids must contain valid UUIDs",
        ));
    }
    Ok(())
}

async fn update_logging_settings(
    principal: Principal,
    State(state): State<ControlState>,
    SafeJson(body): SafeJson<UpdateLoggingSettings>,
) -> ApiResult<Json<LoggingSettings>> {
    require_superadmin(&principal)?;
    validate_settings(&body)?;
    let row = LoggingSettingsRepo(pool(&state))
        .update(
            body.sample_rate,
            body.payload_capture_enabled,
            body.payload_capture_max_bytes,
            &body.payload_capture_redact_fields,
            &body.payload_capture_models,
            &body.payload_capture_virtual_key_ids,
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
            "logging_settings.update",
            Some("logging_settings"),
            None,
            Some(serde_json::json!({
                "sample_rate": row.sample_rate,
                "payload_capture_enabled": row.payload_capture_enabled,
                "payload_capture_max_bytes": row.payload_capture_max_bytes,
                "payload_capture_redact_field_count": row.payload_capture_redact_fields.len(),
                "payload_capture_model_count": row.payload_capture_models.len(),
                "payload_capture_virtual_key_id_count": row.payload_capture_virtual_key_ids.len(),
            })),
        )
        .await
    {
        tracing::warn!(error = %err, "failed to write logging settings audit log");
    }
    Ok(Json(row))
}
