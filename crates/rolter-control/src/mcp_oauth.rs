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
// the one authoritative MCP transport list; the check constraints and the
// dashboard dropdown match it, pinned by `transports_match_the_schema` (#783)
use rolter_core::MCP_TRANSPORTS as TRANSPORTS;
use rolter_store::postgres::models::{
    McpGatewaySettings, McpOAuthGrant, McpOAuthSession, McpServer, McpToolGroup,
};
use rolter_store::postgres::repo::{
    McpGatewaySettingsRepo, McpOAuthRepo, McpServerRepo, McpToolGroupRepo,
};

use crate::crud::{
    log_audit, pool, publish_config_change, require_non_empty, ApiError, ApiResult, SafeJson,
};
use crate::rbac::{authorize, holds_admin, Principal, ScopeChain};
use crate::rbac_matrix::{cap, Requirement};
use crate::ControlState;

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
        .route("/api/v1/orgs/{org_id}/mcp/library", get(list_library))
        .route(
            "/api/v1/orgs/{org_id}/mcp/tool-groups",
            get(list_tool_groups).post(create_tool_group),
        )
        .route(
            "/api/v1/mcp/tool-groups/{id}",
            delete(delete_tool_group).put(update_tool_group),
        )
        .route(
            "/api/v1/orgs/{org_id}/mcp/settings",
            get(get_mcp_settings).put(update_mcp_settings),
        )
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
    #[serde(default)]
    description: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default = "default_custom_source")]
    source: String,
}

fn default_true() -> bool {
    true
}

fn default_custom_source() -> String {
    "custom".to_string()
}

fn validate_tools(tools: &[String]) -> ApiResult<()> {
    if tools.iter().any(|tool| tool.trim().is_empty()) {
        return Err(invalid("tools must not contain empty names"));
    }
    let unique: std::collections::HashSet<_> = tools.iter().collect();
    if unique.len() != tools.len() {
        return Err(invalid("tools must not contain duplicates"));
    }
    Ok(())
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
    validate_tools(&body.tools)?;
    if !matches!(body.source.as_str(), "custom" | "library") {
        return Err(invalid("source must be custom or library"));
    }
    if body.source == "library" {
        let trusted = LIBRARY.iter().any(|item| {
            item.slug == body.slug
                && item.name == body.name
                && item.url == body.url
                && item.transport == transport
                && body
                    .tools
                    .iter()
                    .map(String::as_str)
                    .eq(item.tools.iter().copied())
                && body
                    .required_scopes
                    .iter()
                    .map(String::as_str)
                    .eq(item.required_scopes.iter().copied())
        });
        if !trusted {
            return Err(invalid(
                "library server definition does not match the curated catalog",
            ));
        }
    }
    let server = McpServerRepo(pool(&state))
        .create(
            org_id,
            &body.name,
            &body.slug,
            &body.url,
            transport,
            &body.description,
            body.enabled,
            &body.tools,
            &body.source,
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
    name: Option<String>,
    url: Option<String>,
    transport: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
    tools: Option<Vec<String>>,
    required_scopes: Option<Vec<String>>,
}

async fn update_server(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<Uuid>,
    SafeJson(body): SafeJson<UpdateMcpServer>,
) -> ApiResult<Json<McpServer>> {
    let repo = McpServerRepo(pool(&state));
    let current = repo.get(id).await?;
    authorize(
        &state,
        &principal,
        ScopeChain::org(current.org_id),
        cap!("mcp_server", Update),
    )
    .await?;
    let name = body.name.as_deref().unwrap_or(&current.name);
    let url = body.url.as_deref().unwrap_or(&current.url);
    let transport = body.transport.as_deref().unwrap_or(&current.transport);
    let description = body.description.as_deref().unwrap_or(&current.description);
    let enabled = body.enabled.unwrap_or(current.enabled);
    let tools = body.tools.as_deref().unwrap_or(&current.tools);
    let required_scopes = body
        .required_scopes
        .as_deref()
        .unwrap_or(&current.required_scopes);
    require_non_empty(name, "name")?;
    require_non_empty(url, "url")?;
    if !TRANSPORTS.contains(&transport) {
        return Err(invalid(format!(
            "transport must be one of {}",
            TRANSPORTS.join(", ")
        )));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(invalid("url must be an http(s) endpoint"));
    }
    state
        .egress
        .check_url(url, "MCP server URL")
        .map_err(invalid)?;
    validate_required_scopes(required_scopes)?;
    validate_tools(tools)?;
    let server = repo
        .update(
            id,
            name,
            url,
            transport,
            description,
            enabled,
            tools,
            required_scopes,
        )
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

#[derive(Debug, Clone, Serialize)]
struct LibraryItem {
    slug: &'static str,
    name: &'static str,
    description: &'static str,
    url: &'static str,
    transport: &'static str,
    tools: &'static [&'static str],
    required_scopes: &'static [&'static str],
    installed: bool,
}

struct LibraryDefinition {
    slug: &'static str,
    name: &'static str,
    description: &'static str,
    url: &'static str,
    transport: &'static str,
    tools: &'static [&'static str],
    required_scopes: &'static [&'static str],
}

const LIBRARY: &[LibraryDefinition] = &[
    LibraryDefinition {
        slug: "github",
        name: "GitHub",
        description: "Repository, issue, pull request, and code search tools.",
        url: "https://api.githubcopilot.com/mcp/",
        transport: "streamable_http",
        tools: &[],
        required_scopes: &[],
    },
    LibraryDefinition {
        slug: "sentry",
        name: "Sentry",
        description: "Inspect production issues, traces, and event context.",
        url: "https://mcp.sentry.dev/mcp",
        transport: "streamable_http",
        tools: &[],
        required_scopes: &[],
    },
    LibraryDefinition {
        slug: "notion",
        name: "Notion",
        description: "Search and update pages in an organization workspace.",
        url: "https://mcp.notion.com/mcp",
        transport: "streamable_http",
        tools: &[],
        required_scopes: &[],
    },
    LibraryDefinition {
        slug: "linear",
        name: "Linear",
        description: "Browse and manage teams, issues, and projects.",
        url: "https://mcp.linear.app/mcp",
        transport: "streamable_http",
        tools: &[],
        required_scopes: &[],
    },
];

async fn list_library(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<Vec<LibraryItem>>> {
    authorize(
        &state,
        &principal,
        ScopeChain::org(org_id),
        cap!("mcp_server", Read),
    )
    .await?;
    let installed: std::collections::HashSet<_> = McpServerRepo(pool(&state))
        .list(org_id)
        .await?
        .into_iter()
        .map(|server| server.slug)
        .collect();
    Ok(Json(
        LIBRARY
            .iter()
            .map(|item| LibraryItem {
                slug: item.slug,
                name: item.name,
                description: item.description,
                url: item.url,
                transport: item.transport,
                tools: item.tools,
                required_scopes: item.required_scopes,
                installed: installed.contains(item.slug),
            })
            .collect(),
    ))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ToolRef {
    server_id: Uuid,
    tool: String,
}

#[derive(Debug, Deserialize)]
struct ToolGroupInput {
    name: String,
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    tools: Vec<ToolRef>,
}

fn validate_tool_refs(tools: &[ToolRef]) -> ApiResult<()> {
    if tools.iter().any(|item| item.tool.trim().is_empty()) {
        return Err(invalid("tool group references must name a tool"));
    }
    let unique: std::collections::HashSet<_> = tools
        .iter()
        .map(|item| (item.server_id, item.tool.as_str()))
        .collect();
    if unique.len() != tools.len() {
        return Err(invalid("tool group references must be unique"));
    }
    Ok(())
}

async fn validate_tool_refs_for_org(
    state: &ControlState,
    org_id: Uuid,
    tools: &[ToolRef],
) -> ApiResult<()> {
    validate_tool_refs(tools)?;
    let servers = McpServerRepo(pool(state)).list(org_id).await?;
    for item in tools {
        let server = servers
            .iter()
            .find(|server| server.id == item.server_id)
            .ok_or_else(|| invalid("tool group references a server outside this organization"))?;
        if !server.tools.contains(&item.tool) {
            return Err(invalid(format!(
                "tool '{}' is not declared by server '{}'",
                item.tool, server.slug
            )));
        }
    }
    Ok(())
}

async fn list_tool_groups(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<Vec<McpToolGroup>>> {
    authorize(
        &state,
        &principal,
        ScopeChain::org(org_id),
        cap!("mcp_tool_group", Read),
    )
    .await?;
    Ok(Json(McpToolGroupRepo(pool(&state)).list(org_id).await?))
}

async fn create_tool_group(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
    SafeJson(body): SafeJson<ToolGroupInput>,
) -> ApiResult<Json<McpToolGroup>> {
    authorize(
        &state,
        &principal,
        ScopeChain::org(org_id),
        cap!("mcp_tool_group", Create),
    )
    .await?;
    require_non_empty(&body.name, "name")?;
    require_non_empty(&body.slug, "slug")?;
    validate_tool_refs_for_org(&state, org_id, &body.tools).await?;
    let tools = serde_json::to_value(&body.tools).map_err(|error| invalid(error.to_string()))?;
    let group = McpToolGroupRepo(pool(&state))
        .create(
            org_id,
            &body.name,
            &body.slug,
            &body.description,
            body.enabled,
            &tools,
        )
        .await?;
    log_audit(
        &state,
        &principal,
        Some(org_id),
        "mcp_tool_group.create",
        "mcp_tool_group",
        group.id,
        serde_json::json!({"slug": group.slug}),
    )
    .await;
    Ok(Json(group))
}

#[derive(Debug, Deserialize)]
struct UpdateToolGroup {
    name: String,
    #[serde(default)]
    description: String,
    enabled: bool,
    #[serde(default)]
    tools: Vec<ToolRef>,
}

async fn update_tool_group(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<Uuid>,
    SafeJson(body): SafeJson<UpdateToolGroup>,
) -> ApiResult<Json<McpToolGroup>> {
    require_non_empty(&body.name, "name")?;
    let repo = McpToolGroupRepo(pool(&state));
    let current = repo.get(id).await?;
    authorize(
        &state,
        &principal,
        ScopeChain::org(current.org_id),
        cap!("mcp_tool_group", Update),
    )
    .await?;
    validate_tool_refs_for_org(&state, current.org_id, &body.tools).await?;
    let tools = serde_json::to_value(&body.tools).map_err(|error| invalid(error.to_string()))?;
    let group = repo
        .update(id, &body.name, &body.description, body.enabled, &tools)
        .await?;
    log_audit(
        &state,
        &principal,
        Some(group.org_id),
        "mcp_tool_group.update",
        "mcp_tool_group",
        group.id,
        serde_json::json!({"tools": body.tools.len()}),
    )
    .await;
    Ok(Json(group))
}

async fn delete_tool_group(
    principal: Principal,
    State(state): State<ControlState>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let repo = McpToolGroupRepo(pool(&state));
    let group = repo.get(id).await?;
    authorize(
        &state,
        &principal,
        ScopeChain::org(group.org_id),
        cap!("mcp_tool_group", Delete),
    )
    .await?;
    repo.delete(id).await?;
    log_audit(
        &state,
        &principal,
        Some(group.org_id),
        "mcp_tool_group.delete",
        "mcp_tool_group",
        id,
        serde_json::json!({"slug": group.slug}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_mcp_settings(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<McpGatewaySettings>> {
    authorize(
        &state,
        &principal,
        ScopeChain::org(org_id),
        cap!("mcp_settings", Read),
    )
    .await?;
    Ok(Json(
        McpGatewaySettingsRepo(pool(&state)).get(org_id).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct McpSettingsInput {
    default_transport: String,
    connect_timeout_ms: i32,
    request_timeout_ms: i32,
    max_retries: i32,
    default_failure_mode: String,
    allow_unlisted_tools: bool,
}

async fn update_mcp_settings(
    principal: Principal,
    State(state): State<ControlState>,
    Path(org_id): Path<Uuid>,
    SafeJson(body): SafeJson<McpSettingsInput>,
) -> ApiResult<Json<McpGatewaySettings>> {
    authorize(
        &state,
        &principal,
        ScopeChain::org(org_id),
        cap!("mcp_settings", Update),
    )
    .await?;
    if !matches!(
        body.default_transport.as_str(),
        "sse" | "streamable_http" | "websocket"
    ) {
        return Err(invalid(
            "default_transport must be sse, streamable_http, or websocket",
        ));
    }
    if !(100..=60_000).contains(&body.connect_timeout_ms) {
        return Err(invalid("connect_timeout_ms must be between 100 and 60000"));
    }
    if !(1_000..=300_000).contains(&body.request_timeout_ms) {
        return Err(invalid(
            "request_timeout_ms must be between 1000 and 300000",
        ));
    }
    if !(0..=5).contains(&body.max_retries) {
        return Err(invalid("max_retries must be between 0 and 5"));
    }
    if !matches!(
        body.default_failure_mode.as_str(),
        "fail_open" | "fail_closed"
    ) {
        return Err(invalid(
            "default_failure_mode must be fail_open or fail_closed",
        ));
    }
    let settings = McpGatewaySettingsRepo(pool(&state))
        .update(
            org_id,
            &body.default_transport,
            body.connect_timeout_ms,
            body.request_timeout_ms,
            body.max_retries,
            &body.default_failure_mode,
            body.allow_unlisted_tools,
        )
        .await?;
    log_audit(
        &state,
        &principal,
        Some(org_id),
        "mcp_settings.update",
        "mcp_settings",
        org_id,
        serde_json::json!({"default_transport": settings.default_transport}),
    )
    .await;
    Ok(Json(settings))
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
    fn transports_match_the_schema() {
        // the allowlist, the two check constraints and the dashboard dropdown
        // describe one set. this reads the migrations rather than restating
        // the values, so widening one side without the other fails here (#783)
        let latest = include_str!("../../rolter-store/migrations/0059_mcp_transport_allowlist.sql");
        let settings = include_str!("../../rolter-store/migrations/0055_mcp_management.sql");

        // the SQL tuple each constraint checks against, e.g. "'sse', 'websocket'"
        fn tuple_after(source: &str, needle: &str) -> String {
            let at = source.find(needle).expect("constraint present");
            let rest = &source[at + needle.len()..];
            let open = rest.find('(').expect("open paren");
            let close = rest.find(')').expect("close paren");
            rest[open + 1..close].to_string()
        }

        for tuple in [
            tuple_after(latest, "check (transport in"),
            tuple_after(settings, "check (default_transport in"),
        ] {
            let listed: Vec<String> = tuple
                .split(',')
                .map(|value| value.trim().trim_matches('\'').to_string())
                .collect();
            let expected: Vec<String> = TRANSPORTS.iter().map(|t| t.to_string()).collect();
            assert_eq!(listed, expected, "constraint disagrees with TRANSPORTS");
        }

        // stdio is deliberately gone: a hosted control plane cannot dial a
        // local subprocess, and the HTTP proxy path never served one
        assert!(!TRANSPORTS.contains(&"stdio"));
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

    #[test]
    fn registry_rejects_duplicate_or_empty_tool_names() {
        assert!(validate_tools(&["search".to_string(), "search".to_string()]).is_err());
        assert!(validate_tools(&[" ".to_string()]).is_err());
        assert!(validate_tools(&["search".to_string(), "create_issue".to_string()]).is_ok());
    }

    #[test]
    fn legacy_scope_only_server_patch_remains_valid() {
        let patch: UpdateMcpServer = serde_json::from_value(serde_json::json!({
            "required_scopes": ["repo"]
        }))
        .unwrap();
        assert_eq!(patch.required_scopes, Some(vec!["repo".to_string()]));
        assert!(patch.name.is_none());
        assert!(patch.enabled.is_none());
    }

    #[test]
    fn tool_groups_reject_duplicate_server_tool_pairs() {
        let server_id = Uuid::new_v4();
        let duplicate = vec![
            ToolRef {
                server_id,
                tool: "search".to_string(),
            },
            ToolRef {
                server_id,
                tool: "search".to_string(),
            },
        ];
        assert!(validate_tool_refs(&duplicate).is_err());
        assert!(validate_tool_refs(&[ToolRef {
            server_id,
            tool: "search".to_string()
        }])
        .is_ok());
    }

    #[test]
    fn curated_library_slugs_and_tools_are_unique() {
        let mut slugs = std::collections::HashSet::new();
        for item in LIBRARY {
            assert!(
                slugs.insert(item.slug),
                "duplicate library slug: {}",
                item.slug
            );
            assert!(item.url.starts_with("https://"));
            assert!(TRANSPORTS.contains(&item.transport));
            assert!(validate_tools(
                &item
                    .tools
                    .iter()
                    .map(|tool| (*tool).to_string())
                    .collect::<Vec<_>>()
            )
            .is_ok());
            assert!(validate_required_scopes(
                &item
                    .required_scopes
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect::<Vec<_>>()
            )
            .is_ok());
        }
    }
}
