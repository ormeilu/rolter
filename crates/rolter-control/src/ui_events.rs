//! Dashboard UX event ingestion (#805).
//!
//! Browser spans say how long a screen took and whether it errored. They do not
//! say whether it was *usable*: which screens are slow to become interactive,
//! where people back out, which forms get abandoned, which error states are
//! actually reached. This is the endpoint behind that stream, writing to the
//! `ui_events` ClickHouse table next to `request_logs` — same deployment, same
//! retention, same redaction, and joinable against gateway traffic on
//! `trace_id`.
//!
//! Two properties are worth stating plainly, because they are what make it safe
//! to point a browser at this endpoint:
//!
//! - **Events are structural only.** Every accepted field is a key, an enum, a
//!   duration or an id. There is no field a form value, prompt or free-text
//!   body could travel in, so the schema is the guarantee rather than a policy
//!   applied on write. A validation *rule name* is accepted; the value that
//!   failed it is not.
//! - **Attribution is server-side.** `user_id` is taken from the authenticated
//!   principal and a client-supplied one is ignored, so a caller cannot file
//!   events against somebody else.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::analytics::client_or_503;
use crate::auth::CurrentUser;
use crate::crud::{ApiError, ApiResult};
use crate::ControlState;

/// The `action` enum in `008_ui_events.sql`. ClickHouse rejects a value outside
/// its `Enum8`, but failing here gives the caller a 400 naming the field
/// instead of a 500 from the database.
const ACTIONS: &[&str] = &[
    "screen_view",
    "time_to_interactive",
    "navigate",
    "back_out",
    "form_submit",
    "form_abandon",
    "validation_error",
    "empty_state",
    "error_state",
    "save_confirmed",
];

/// The `outcome` enum in the same migration.
const OUTCOMES: &[&str] = &["ok", "error", "cancelled"];

/// Browsers batch these — a `visibilitychange` beacon flushes everything queued
/// since the last one — so a batch is the normal shape, not an optimization.
/// The cap keeps one call from turning into an unbounded insert.
const MAX_BATCH: usize = 100;

/// `screen`, `target` and `from_screen` are `LowCardinality` columns: they are
/// meant to hold a bounded set of stable keys. This bound is deliberately tight
/// enough that a URL with ids and query parameters in it will not fit, which is
/// the mistake most likely to turn a key column into a free-text one.
const MAX_KEY_LEN: usize = 96;

pub(crate) fn router() -> Router<ControlState> {
    Router::new().route("/api/v1/ui-events", post(ingest))
}

#[derive(Debug, Deserialize)]
struct Batch {
    events: Vec<UiEvent>,
}

#[derive(Debug, Deserialize)]
struct UiEvent {
    event_id: String,
    /// stable screen key (a route id), never a URL
    screen: String,
    action: String,
    #[serde(default)]
    outcome: Option<String>,
    /// the name of the form, control or validation rule — never its value
    #[serde(default)]
    target: String,
    #[serde(default)]
    from_screen: String,
    #[serde(default)]
    duration_ms: u64,
    /// links the event to the gateway request it caused; empty when the
    /// interaction issued no request, or when browser tracing is off
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    org_id: String,
    #[serde(default)]
    team_id: String,
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    app_version: String,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::Core(rolter_core::Error::Config(message.into()))
}

/// A bounded, control-character-free key. Empty is allowed for the optional
/// columns; the caller decides which fields are required.
fn validate_key(value: &str, name: &str, required: bool) -> Result<(), String> {
    if value.is_empty() {
        return if required {
            Err(format!("{name} is required"))
        } else {
            Ok(())
        };
    }
    if value.len() > MAX_KEY_LEN {
        return Err(format!("{name} must be at most {MAX_KEY_LEN} characters"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{name} must not contain control characters"));
    }
    Ok(())
}

fn validate(event: &UiEvent) -> Result<(), String> {
    validate_key(&event.event_id, "event_id", true)?;
    validate_key(&event.screen, "screen", true)?;
    validate_key(&event.target, "target", false)?;
    validate_key(&event.from_screen, "from_screen", false)?;
    validate_key(&event.session_id, "session_id", false)?;
    validate_key(&event.trace_id, "trace_id", false)?;
    validate_key(&event.app_version, "app_version", false)?;
    validate_key(&event.org_id, "org_id", false)?;
    validate_key(&event.team_id, "team_id", false)?;
    validate_key(&event.project_id, "project_id", false)?;
    if !ACTIONS.contains(&event.action.as_str()) {
        return Err(format!("action must be one of {ACTIONS:?}"));
    }
    if let Some(outcome) = &event.outcome {
        if !OUTCOMES.contains(&outcome.as_str()) {
            return Err(format!("outcome must be one of {OUTCOMES:?}"));
        }
    }
    if event.duration_ms > u32::MAX.into() {
        return Err("duration_ms exceeds UInt32 range".to_string());
    }
    Ok(())
}

/// Build the ClickHouse row. `user_id` is threaded in from the authenticated
/// principal rather than read off the event, so attribution cannot be forged.
fn row(event: &UiEvent, user_id: &str) -> Value {
    json!({
        "event_id": event.event_id,
        "trace_id": event.trace_id,
        "session_id": event.session_id,
        "org_id": event.org_id,
        "team_id": event.team_id,
        "project_id": event.project_id,
        "user_id": user_id,
        "screen": event.screen,
        "action": event.action,
        "target": event.target,
        // the column is not nullable; 'ok' is the neutral value for the
        // actions that carry no outcome of their own, such as screen_view
        "outcome": event.outcome.as_deref().unwrap_or("ok"),
        "duration_ms": event.duration_ms as u32,
        "from_screen": event.from_screen,
        "app_version": event.app_version,
    })
}

async fn ingest(
    caller: CurrentUser,
    State(state): State<ControlState>,
    Json(batch): Json<Batch>,
) -> ApiResult<StatusCode> {
    // holding a `CurrentUser` is the whole check, as in `me.rs`: this is the
    // dashboard reporting on its own user's session, not an operator surface.
    // There is deliberately no capability row — the RBAC model holds that a
    // viewer writes nothing, and a viewer who could not file their own screen
    // views would simply be missing from every funnel

    // deployment-level opt-out, checked before validation so a deployment that
    // wants none of this is not also a deployment that argues about payloads
    if !state.store.load().await?.logging.ui_events {
        return Ok(StatusCode::ACCEPTED);
    }

    if batch.events.is_empty() {
        return Ok(StatusCode::ACCEPTED);
    }
    if batch.events.len() > MAX_BATCH {
        return Err(invalid(format!(
            "batch holds {} events; the limit is {MAX_BATCH}",
            batch.events.len()
        )));
    }
    for event in &batch.events {
        validate(event).map_err(invalid)?;
    }

    let user_id = caller.user.id.to_string();
    let rows: Vec<Value> = batch.events.iter().map(|e| row(e, &user_id)).collect();

    let ch = client_or_503(&state).map_err(|_| {
        ApiError::Core(rolter_core::Error::Store(
            "UX event ingestion requires CLICKHOUSE_URL".to_string(),
        ))
    })?;
    ch.insert_ui_events(&rows)
        .await
        .map_err(|err| ApiError::Core(rolter_core::Error::Store(err.to_string())))?;
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> UiEvent {
        UiEvent {
            event_id: "01J8Z0".to_string(),
            screen: "models".to_string(),
            action: "screen_view".to_string(),
            outcome: None,
            target: String::new(),
            from_screen: String::new(),
            duration_ms: 0,
            trace_id: String::new(),
            session_id: String::new(),
            org_id: String::new(),
            team_id: String::new(),
            project_id: String::new(),
            app_version: String::new(),
        }
    }

    #[test]
    fn accepts_a_minimal_event() {
        assert!(validate(&event()).is_ok());
    }

    #[test]
    fn every_migration_action_is_accepted() {
        // drift here would 500 on the ClickHouse Enum8 instead of 400ing
        for action in ACTIONS {
            let mut e = event();
            e.action = (*action).to_string();
            assert!(validate(&e).is_ok(), "{action} rejected");
        }
    }

    #[test]
    fn rejects_an_unknown_action_and_outcome() {
        let mut e = event();
        "sudo_make_me_a_sandwich".clone_into(&mut e.action);
        assert!(validate(&e).is_err());

        let mut e = event();
        e.outcome = Some("maybe".to_string());
        assert!(validate(&e).is_err());
    }

    #[test]
    fn requires_the_identifying_fields() {
        let mut e = event();
        e.event_id = String::new();
        assert!(validate(&e).is_err());

        let mut e = event();
        e.screen = String::new();
        assert!(validate(&e).is_err());
    }

    #[test]
    fn a_url_shaped_screen_key_does_not_fit() {
        // the failure mode this bound exists for: a real URL carries ids and
        // query parameters, which would make a LowCardinality column unbounded
        let mut e = event();
        e.screen = format!(
            "https://rolter.example/models/{}?tab=targets",
            "x".repeat(80)
        );
        assert!(validate(&e).is_err());
    }

    #[test]
    fn rejects_control_characters_in_a_key() {
        let mut e = event();
        "models\nrows".clone_into(&mut e.target);
        assert!(validate(&e).is_err());
    }

    #[test]
    fn duration_must_fit_the_uint32_column() {
        let mut e = event();
        e.duration_ms = u64::from(u32::MAX) + 1;
        assert!(validate(&e).is_err());
    }

    #[test]
    fn the_row_takes_user_id_from_the_principal_not_the_event() {
        let built = row(&event(), "11111111-2222-3333-4444-555555555555");
        assert_eq!(
            built["user_id"],
            json!("11111111-2222-3333-4444-555555555555")
        );
        // and an action with no outcome of its own still fills the column
        assert_eq!(built["outcome"], json!("ok"));
    }

    #[test]
    fn the_row_carries_every_column_the_table_declares() {
        let built = row(&event(), "u");
        for column in [
            "event_id",
            "trace_id",
            "session_id",
            "org_id",
            "team_id",
            "project_id",
            "user_id",
            "screen",
            "action",
            "target",
            "outcome",
            "duration_ms",
            "from_screen",
            "app_version",
        ] {
            assert!(built.get(column).is_some(), "row is missing {column}");
        }
    }
}
