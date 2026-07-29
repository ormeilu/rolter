//! Row types returned by the repository layer. These mirror `migrations/`
//! column-for-column; domain interpretation (e.g. parsing `strategy` into
//! [`rolter_core::BalancingStrategy`]) is left to callers such as the
//! control-plane API and [`super::PostgresConfigStore`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Org {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Team {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub team_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BusinessUnit {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub slug: String,
    pub retired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Customer {
    pub id: Uuid,
    pub org_id: Uuid,
    pub business_unit_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub retired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub published_version: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PromptTemplateVersion {
    pub template_id: Uuid,
    pub version: i32,
    pub variables: serde_json::Value,
    pub decorators: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PromptTemplateScope {
    pub template_id: Uuid,
    pub version: i32,
    pub scope_type: String,
    pub scope_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Skill {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub retired_at: Option<DateTime<Utc>>,
    pub published_version: Option<i32>,
    pub allowed_team_ids: Vec<Uuid>,
    pub minimum_role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SkillVersion {
    pub skill_id: Uuid,
    pub version: i32,
    pub content: Option<String>,
    pub content_ref: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Provider {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// stable, URL-safe identity for `provider-slug/model` addressing;
    /// `unique(org_id, slug)` and immutable by default
    pub slug: String,
    /// a supported provider kind such as `openai`, `ollama`, `openrouter`, or `tei`
    pub kind: String,
    pub api_base: String,
    pub api_key_env: Option<String>,
    pub egress_proxy: Option<String>,
    pub egress_proxies: sqlx::types::Json<Vec<String>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub project_id: Uuid,
    pub model: String,
    /// one of `round_robin` | `random` | `power_of_two` | `consistent_hash` | `cache_aware` | `weighted` | `pipeline`
    pub strategy: String,
    pub enabled: bool,
    /// admin default inference params (jsonb object); mirrors config `[routes.params]`
    pub params: serde_json::Value,
    /// override policy (jsonb `{mode, allow, deny}`); mirrors config `[routes.param_policy]`
    pub param_policy: serde_json::Value,
    /// catalog metadata and per-model execution policy; mirrors `[routes.advanced]`
    pub advanced: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct RouteTarget {
    pub id: Uuid,
    pub route_id: Uuid,
    pub provider_id: Uuid,
    pub upstream_model: Option<String>,
    pub weight: i32,
    pub created_at: DateTime<Utc>,
}

/// A provider group: a fleet of providers addressable as `group-slug/model`
/// (ADR-0017 addendum, ADR-0022). Org-scoped; the slug shares the provider slug
/// namespace.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProviderGroup {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// stable, URL-safe identity; `unique(org_id, slug)`, immutable by default
    pub slug: String,
    /// one of the balancing-strategy keys (`round_robin`, `weighted`, …)
    pub strategy: String,
    pub created_at: DateTime<Utc>,
}

/// One membership row of a [`ProviderGroup`]. `provider_name` is joined in for
/// config assembly; `upstream_model` null means passthrough of the requested
/// model.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProviderGroupMember {
    pub group_id: Uuid,
    pub provider_id: Uuid,
    pub provider_name: String,
    pub upstream_model: Option<String>,
    pub weight: i32,
    pub position: i32,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct VirtualKey {
    pub id: Uuid,
    pub project_id: Uuid,
    pub key_hash: String,
    pub key_prefix: String,
    pub name: Option<String>,
    pub models: Vec<String>,
    /// empty means the key may reach every provider on an allowed route
    pub providers: Vec<String>,
    pub disabled: bool,
    pub expires_at: Option<DateTime<Utc>>,
    /// per-key response-cache override; `NULL` inherits the route decision
    pub cache_enabled: Option<bool>,
    /// local account that minted this key via the self-service panel; `NULL`
    /// for admin-created or bootstrap-config keys (ROL-224)
    pub created_by: Option<Uuid>,
    /// business unit this key's spend rolls up to; `NULL` leaves the key
    /// attributed to its tenancy chain only
    pub business_unit_id: Option<Uuid>,
    /// customer this key's spend rolls up to; `NULL` when unattributed
    pub customer_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// a virtual key owned by the current user, enriched with the project/org names
/// it belongs to so the self-service panel can label it without needing admin
/// read access to the tenancy tables. never carries the key hash.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct OwnedVirtualKey {
    pub id: Uuid,
    pub project_id: Uuid,
    pub project_name: String,
    pub org_name: String,
    pub key_prefix: String,
    pub name: Option<String>,
    pub models: Vec<String>,
    pub disabled: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Budget {
    pub id: Uuid,
    /// one of `org` | `team` | `project` | `virtual_key`
    pub scope_type: String,
    pub scope_id: Uuid,
    /// decimal(12,4), returned as text to avoid a numeric-crate dependency
    pub limit_usd: String,
    pub period: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct RateLimit {
    pub id: Uuid,
    pub scope_type: String,
    pub scope_id: Uuid,
    pub rpm: Option<i32>,
    pub tpm: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ModelPrice {
    pub id: Uuid,
    pub model: String,
    /// decimal(12,6), returned as text to avoid a numeric-crate dependency
    pub input_per_mtok: String,
    pub output_per_mtok: String,
    pub cached_input_per_mtok: Option<String>,
    pub currency: String,
    pub created_at: DateTime<Utc>,
}
/// singleton persisted feature flags that gate supported subsystems
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct FeatureFlags {
    pub response_cache: bool,
    pub cache_aware_routing: bool,
    pub circuit_breaker: bool,
    pub active_health_checks: bool,
    pub complexity_routing: bool,
    pub guardrails: bool,
    pub updated_at: DateTime<Utc>,
}

/// singleton persisted runtime policy projected into snapshots
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct RuntimePolicy {
    pub retry_max_retries: i32,
    pub retry_base_ms: i32,
    pub retry_max_ms: i32,
    pub timeout_connect_s: i32,
    pub timeout_request_s: i32,
    pub queue_enabled: bool,
    pub queue_capacity: i32,
    pub queue_workers: i32,
    pub queue_backpressure: String,
    pub queue_block_ms: i32,
    pub updated_at: DateTime<Utc>,
}

/// a gateway or control node seen by the control plane, refreshed on every
/// snapshot poll it makes
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ClusterNode {
    pub id: String,
    pub role: String,
    pub build_version: String,
    /// config snapshot version the node reported running
    pub config_version: i64,
    /// operator-requested state: `active` or `draining`
    pub desired_state: String,
    pub state_changed_at: DateTime<Utc>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// singleton persisted cross-dialect compatibility policy projected into
/// snapshots
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CompatibilityPolicy {
    pub anthropic_version: String,
    pub default_max_tokens: i32,
    pub updated_at: DateTime<Utc>,
}

/// an MCP server an org has registered; the anchor OAuth grants hang off
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct McpServer {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub slug: String,
    pub url: String,
    /// one of `stdio` | `sse` | `streamable_http` | `websocket`
    pub transport: String,
    pub created_at: DateTime<Utc>,
}

/// a user's consent grant against one MCP server. Revoked grants are kept so
/// the audit trail survives; `revoked_at` is the live/dead flag.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct McpOAuthGrant {
    pub id: Uuid,
    pub server_id: Uuid,
    pub user_id: Uuid,
    pub scopes: Vec<String>,
    pub granted_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<Uuid>,
}

/// Session metadata for a grant, with **no** token material on it. The sealed
/// tokens live in the same row but are only ever read through
/// [`super::repo::McpOAuthRepo::open_session`], so a DTO that reaches an API
/// response cannot carry them by accident.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct McpOAuthSession {
    pub id: Uuid,
    pub grant_id: Uuid,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub refresh_expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// whether a refresh token is stored, so a UI can show renewability
    /// without the control plane ever handing the token out
    pub has_refresh_token: bool,
}

/// singleton persisted adaptive-routing policy projected into snapshots
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AdaptiveRoutingPolicy {
    pub enabled: bool,
    pub latency_weight: f32,
    pub cost_weight: f32,
    pub load_weight: f32,
    pub exploration_ratio: f32,
    pub min_samples: i32,
    pub updated_at: DateTime<Utc>,
}

/// singleton persisted request-log policy projected into snapshots
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct LoggingSettings {
    pub sample_rate: f64,
    pub payload_capture_enabled: bool,
    pub payload_capture_max_bytes: i32,
    pub payload_capture_redact_fields: Vec<String>,
    pub payload_capture_models: Vec<String>,
    pub payload_capture_virtual_key_ids: Vec<String>,
    /// how long request-log metadata is kept in clickhouse
    pub retention_days: i32,
    /// how long captured raw payloads are kept; always the shorter clock
    pub payload_retention_hours: i32,
    pub updated_at: DateTime<Utc>,
}

/// a local account. `password_hash` is `None` for sso-only users (a later
/// phase) and is never serialized back to a client
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
    pub is_superadmin: bool,
    /// set when an admin deactivates the account; a non-null value blocks login
    /// while keeping the row, memberships and audit trail intact
    pub deactivated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// a role grant at a scope; scope is the most specific non-null id among
/// `org_id`/`team_id`/`project_id`
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Membership {
    pub id: Uuid,
    pub user_id: Uuid,
    pub org_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    /// one of `admin` | `member` | `viewer`
    pub role: String,
    pub created_at: DateTime<Utc>,
}

/// a record of an admin/CRUD/auth action, for the audit-log API
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: Uuid,
    pub org_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<Uuid>,
    pub detail: Option<serde_json::Value>,
    pub at: DateTime<Utc>,
}

/// Global control-plane security settings. Managed dashboard credentials are
/// encrypted separately and intentionally never appear on this DTO.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SecuritySettings {
    pub virtual_key_required: bool,
    pub allow_direct_provider_keys: bool,
    pub allowed_origins: Vec<String>,
    pub allowed_headers: Vec<String>,
    pub required_headers: serde_json::Value,
    pub auth_bypass_routes: Vec<String>,
    pub dashboard_auth_enabled: bool,
    pub dashboard_credential_ref: Option<String>,
    pub dashboard_secret_configured: bool,
    pub updated_at: DateTime<Utc>,
}

/// a login session. `token_hash` is the peppered digest of the opaque bearer
/// token handed to the client; the plaintext token is never stored
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}
