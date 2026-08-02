//! MCP servers, OAuth consent grants and token sessions (#541).
//!
//! Three rules shape this module:
//!
//! 1. **No token material crosses the API boundary.** Access and refresh
//!    tokens are sealed with the deployment KEK in [`rolter_store`] and are
//!    only readable through `McpOAuthRepo::open_session`, which nothing here
//!    calls. Responses carry metadata and a `has_refresh_token` flag.
//! 2. **A grant is the unit of consent.** Revoking one revokes every session
//!    under it in the same transaction, so withdrawn consent cannot leave a
//!    live token behind.
//! 3. **A user sees their own.** Org admins see every grant and session in
//!    the org; a member or viewer sees only the ones they own, and may revoke
//!    only those — cross-tenant reads are impossible because every listing is
//!    joined through the org.
//!
//! Transport and the authorization-code/OBO exchange live with the MCP proxy
//! (#423); this module owns persistence, listing, revocation and audit.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use rolter_core::Error;
use rolter_store::postgres::models::{McpOAuthGrant, McpOAuthSession, McpServer};
use rolter_store::postgres::repo::{McpOAuthRepo, McpServerRepo};

use crate::crud::{
    log_audit, pool, publish_config_change, require_non_empty, ApiError, ApiResult, SafeJson,
};
use crate::rbac::{authorize, holds_admin, Principal, ScopeChain};
use crate::rbac_matrix::{cap, Requirement};
use crate::ControlState;

/// MCP transports a registered server may speak. Mirrors the set the tool-call
/// log already accepts, so both screens describe the same world.
const TRANSPORTS: &[&str] = &["stdio", "sse", "streamable_http", "websocket"];

pub(crate) fn router() -> Router<ControlState> {
    Router::new()
        .route(
            "/api/v1/orgs/{org_id}/mcp-servers",
            get(list_servers).post(create_server),
        )
        .route(
            "/api/v1/mcp-servers/{id}",
            delete(delete_server).patch(update_server),
        )
        .route("/api/v1/orgs/{org_id}/mcp/grants", get(list_grants))
        .route("/api/v1/mcp/grants/{id}", delete(revoke_grant))
        .route("/api/v1/orgs/{org_id}/mcp/sessions", get(list_sessions))
        .route("/api/v1/mcp/sessions/{id}", delete(revoke_session))
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::Core(Error::Config(message.into()))
}

fn validate_required_scopes(scopes: &[String]) -> ApiResult<()> {
    if scopes.iter().any(|scope| scope.trim().is_empty()) {
        return Err(invalid("required_scopes must not contain empty values"));
    }
    let unique_scopes: std::collections::HashSet<_> = scopes.iter().collect();
    if unique_scopes.len() != scopes.len() {
        return Err(invalid("required_scopes must not contain duplicates"));
    }
    Ok(())
}

/// Whether the caller holds org admin. Used to decide between "everything in
/// the org" and "only what I own"; a caller who is neither is rejected by the
/// viewer check that runs first. A question, not a guard — the guard is the
/// capability the handler names.
async fn is_org_admin(
    state: &ControlState,
    principal: &Principal,
    org_id: Uuid,
) -> ApiResult<bool> {
    holds_admin(state, principal, ScopeChain::org(org_id)).await
}

/// The owner filter for a listing: `None` means "no filter" (org admins),
/// `Some(user)` restricts to the caller's own rows.
async fn owner_filter(
    state: &ControlState,
    principal: &Principal,
    org_id: Uuid,
    requirement: Requirement,
) -> ApiResult<Option<Uuid>> {
    // any membership at the org is required before anything is listed
    authorize(state, principal, ScopeChain::org(org_id), requirement).await?;
    if is_org_admin(state, principal, org_id).await? {
        return Ok(None);
    }
    Ok(match principal {
        Principal::Superadmin => None,
        Principal::User(user) => Some(user.id),
    })
}

async fn list_servers(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<Vec<McpServer>>> {
    authorize(
        &state,
        &principal,
        ScopeChain::org(org_id),
        cap!("mcp_server", Read),
    )
    .await?;
    Ok(Json(McpServerRepo(pool(&state)).list(org_id).await?))
}

#[derive(Debug, Deserialize)]
struct CreateMcpServer {
    name: String,
    slug: String,
    url: String,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    required_scopes: Vec<String>,
}

async fn create_server(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
    SafeJson(body): SafeJson<CreateMcpServer>,
) -> ApiResult<Json<McpServer>> {
    authorize(
        &state,
        &principal,
        ScopeChain::org(org_id),
        cap!("mcp_server", Create),
    )
    .await?;
    require_non_empty(&body.name, "name")?;
    require_non_empty(&body.slug, "slug")?;
    require_non_empty(&body.url, "url")?;
    let transport = body.transport.as_deref().unwrap_or("streamable_http");
    if !TRANSPORTS.contains(&transport) {
        return Err(invalid(format!(
            "transport must be one of {}",
            TRANSPORTS.join(", ")
        )));
    }
    // an MCP server is a URL the deployment will be asked to call on a user's
    // behalf, so it is held to the same egress rules as an upstream provider
    if !(body.url.starts_with("http://") || body.url.starts_with("https://")) {
        return Err(invalid("url must be an http(s) endpoint"));
    }
    state
        .egress
        .check_url(&body.url, "MCP server URL")
        .map_err(invalid)?;
    validate_required_scopes(&body.required_scopes)?;
    let server = McpServerRepo(pool(&state))
        .create(
            org_id,
            &body.name,
            &body.slug,
            &body.url,
            transport,
            &body.required_scopes,
        )
        .await?;
    publish_config_change(&state).await?;
    log_audit(
        &state,
        &principal,
        Some(org_id),
        "mcp_server.create",
        "mcp_server",
        server.id,
        serde_json::json!({"slug": server.slug, "transport": server.transport}),
    )
    .await;
    Ok(Json(server))
}

#[derive(Debug, Deserialize)]
struct UpdateMcpServer {
    required_scopes: Vec<String>,
}

async fn update_server(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<Uuid>,
    SafeJson(body): SafeJson<UpdateMcpServer>,
) -> ApiResult<Json<McpServer>> {
    validate_required_scopes(&body.required_scopes)?;
    let repo = McpServerRepo(pool(&state));
    let current = repo.get(id).await?;
    authorize(
        &state,
        &principal,
        ScopeChain::org(current.org_id),
        cap!("mcp_server", Update),
    )
    .await?;
    let server = repo
        .update_required_scopes(id, &body.required_scopes)
        .await?;
    publish_config_change(&state).await?;
    log_audit(
        &state,
        &principal,
        Some(server.org_id),
        "mcp_server.update",
        "mcp_server",
        server.id,
        serde_json::json!({"required_scopes": server.required_scopes}),
    )
    .await;
    Ok(Json(server))
}

async fn delete_server(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let repo = McpServerRepo(pool(&state));
    let server = repo.get(id).await?;
    authorize(
        &state,
        &principal,
        ScopeChain::org(server.org_id),
        cap!("mcp_server", Delete),
    )
    .await?;
    repo.delete(id).await?;
    publish_config_change(&state).await?;
    log_audit(
        &state,
        &principal,
        Some(server.org_id),
        "mcp_server.delete",
        "mcp_server",
        id,
        serde_json::json!({"slug": server.slug}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// A grant with its live/dead state resolved, so a client never has to infer
/// it from a nullable timestamp.
#[derive(Debug, Serialize)]
struct GrantView {
    #[serde(flatten)]
    grant: McpOAuthGrant,
    active: bool,
}

impl From<McpOAuthGrant> for GrantView {
    fn from(grant: McpOAuthGrant) -> Self {
        let active = grant.revoked_at.is_none();
        Self { grant, active }
    }
}

async fn list_grants(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<Vec<GrantView>>> {
    let owner = owner_filter(&state, &principal, org_id, cap!("mcp_oauth_grant", Read)).await?;
    Ok(Json(
        McpOAuthRepo(pool(&state))
            .list_grants(org_id, owner)
            .await?
            .into_iter()
            .map(GrantView::from)
            .collect(),
    ))
}

/// Whether the caller may revoke something owned by `owner_id` in `org_id`:
/// an org admin, or the owner themself. Anyone else gets `403` — including a
/// member of a *different* org, whose authorize check fails first.
async fn may_revoke(
    state: &ControlState,
    principal: &Principal,
    org_id: Uuid,
    owner_id: Uuid,
    requirement: Requirement,
) -> ApiResult<()> {
    if is_org_admin(state, principal, org_id).await? {
        return Ok(());
    }
    authorize(state, principal, ScopeChain::org(org_id), requirement).await?;
    match principal {
        Principal::Superadmin => Ok(()),
        Principal::User(user) if user.id == owner_id => Ok(()),
        Principal::User(_) => Err(ApiError::Forbidden),
    }
}

async fn revoke_grant(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<GrantView>> {
    let repo = McpOAuthRepo(pool(&state));
    let grant = repo.get_grant(id).await?;
    let server = McpServerRepo(pool(&state)).get(grant.server_id).await?;
    may_revoke(
        &state,
        &principal,
        server.org_id,
        grant.user_id,
        cap!("mcp_oauth_grant", Delete),
    )
    .await?;
    let actor = match &principal {
        Principal::User(user) => Some(user.id),
        Principal::Superadmin => None,
    };
    // revoking the consent revokes its sessions in the same transaction
    let revoked = repo.revoke_grant(id, actor).await?;
    publish_config_change(&state).await?;
    log_audit(
        &state,
        &principal,
        Some(server.org_id),
        "mcp_oauth_grant.revoke",
        "mcp_oauth_grant",
        id,
        serde_json::json!({"server_id": server.id, "user_id": grant.user_id}),
    )
    .await;
    Ok(Json(revoked.into()))
}

async fn list_sessions(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<Vec<McpOAuthSession>>> {
    let owner = owner_filter(&state, &principal, org_id, cap!("mcp_oauth_session", Read)).await?;
    Ok(Json(
        McpOAuthRepo(pool(&state))
            .list_sessions(org_id, owner)
            .await?,
    ))
}

async fn revoke_session(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<McpOAuthSession>> {
    let repo = McpOAuthRepo(pool(&state));
    let session = repo.get_session(id).await?;
    let grant = repo.get_grant(session.grant_id).await?;
    let server = McpServerRepo(pool(&state)).get(grant.server_id).await?;
    may_revoke(
        &state,
        &principal,
        server.org_id,
        grant.user_id,
        cap!("mcp_oauth_session", Delete),
    )
    .await?;
    let revoked = repo.revoke_session(id).await?;
    publish_config_change(&state).await?;
    log_audit(
        &state,
        &principal,
        Some(server.org_id),
        "mcp_oauth_session.revoke",
        "mcp_oauth_session",
        id,
        serde_json::json!({"grant_id": grant.id, "user_id": grant.user_id}),
    )
    .await;
    Ok(Json(revoked))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transports_match_the_tool_call_log() {
        // both screens must describe the same set, or a server registered here
        // could never be attributed to its logged tool calls
        assert_eq!(TRANSPORTS.len(), 4);
        for transport in ["stdio", "sse", "streamable_http", "websocket"] {
            assert!(TRANSPORTS.contains(&transport));
        }
    }

    #[test]
    fn grant_view_derives_active_from_revocation() {
        let mut grant = McpOAuthGrant {
            id: Uuid::nil(),
            server_id: Uuid::nil(),
            user_id: Uuid::nil(),
            scopes: vec!["tools:read".to_string()],
            granted_at: chrono::Utc::now(),
            revoked_at: None,
            revoked_by: None,
        };
        assert!(GrantView::from(grant.clone()).active);
        grant.revoked_at = Some(chrono::Utc::now());
        assert!(!GrantView::from(grant).active);
    }
}
