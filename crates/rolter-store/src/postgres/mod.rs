//! Postgres-backed [`ConfigStore`], gated behind the `postgres` feature.

pub mod crypto;
pub mod models;
pub mod repo;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rolter_core::{
    BackpressurePolicy, BalancingStrategy, BudgetConfig, BudgetPeriod, BudgetScope, Decorator,
    Error, FeatureFlagsConfig, GatewayConfig, GroupMember, McpOAuthSessionConfig, McpServerConfig,
    ModelPriceConfig, ModelRoute, PromptTemplate, PromptTemplateActivationScope,
    PromptTemplatesConfig, ProviderConfig, ProviderGroupConfig, ProviderKind, RateLimitConfig,
    Result, Target, TemplateVariable, VirtualKeyRecord,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::{FromRow, PgPool};
use std::collections::HashMap;
use uuid::Uuid;

use crate::postgres::models::{
    AdaptiveRoutingPolicy, Budget, CompatibilityPolicy, FeatureFlags, LoggingSettings, ModelPrice,
    RateLimit, RuntimePolicy,
};
use crate::ConfigStore;

fn store_err(err: sqlx::Error) -> Error {
    Error::Store(err.to_string())
}

/// Connect to Postgres with a small pool sized for the control plane.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
        .map_err(store_err)
}

/// Run pending migrations against `pool`. The migration set lives in this
/// crate's own `migrations/` directory so it is embedded at compile time and
/// packaged with the published crate; `docker-compose` mounts the same dir for
/// its initdb bootstrap.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|err| Error::Store(err.to_string()))
}

/// Test-only helpers for building isolated, migrated pools. Every test gets its
/// own schema pinned via `search_path`, so plain `cargo test` (which runs tests
/// as threads in one process — e.g. the coverage job) never races on a shared
/// `public` schema during DDL.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicU32, Ordering};

    use sqlx::PgPool;

    use super::{connect, run_migrations};

    static SCHEMA_SEQ: AtomicU32 = AtomicU32::new(0);

    /// Isolated schema name unique to this process and call, safe to
    /// interpolate (only ascii digits and underscores).
    fn unique_schema() -> String {
        let n = SCHEMA_SEQ.fetch_add(1, Ordering::Relaxed);
        format!("test_{}_{}", std::process::id(), n)
    }

    /// `url` with the connection pinned to `schema` via `search_path`, so
    /// migrations and queries land in the isolated schema rather than `public`.
    fn with_search_path(url: &str, schema: &str) -> String {
        let sep = if url.contains('?') { '&' } else { '?' };
        // percent-encode the space and `=` inside the libpq options string
        format!("{url}{sep}options=-c%20search_path%3D{schema}")
    }

    /// Create a fresh isolated schema and return a migrated pool scoped to it.
    pub(crate) async fn fresh_scoped_pool(url: &str) -> PgPool {
        let schema = unique_schema();

        // (re)create the isolated schema over a default-search_path connection
        let admin = connect(url).await.expect("connect");
        sqlx::query(&format!("drop schema if exists {schema} cascade"))
            .execute(&admin)
            .await
            .expect("reset schema");
        sqlx::query(&format!("create schema {schema}"))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        // app pool scoped to the isolated schema so migrations run there
        let pool = connect(&with_search_path(url, &schema))
            .await
            .expect("connect scoped");
        run_migrations(&pool).await.expect("run migrations");
        pool
    }
}

#[derive(FromRow)]
struct ProviderRow {
    name: String,
    slug: String,
    kind: String,
    api_base: String,
    api_key_env: Option<String>,
    egress_proxy: Option<String>,
    egress_proxies: serde_json::Value,
    /// sealed runtime credential from `provider_keys`, when one is stored
    ciphertext: Option<Vec<u8>>,
    nonce: Option<Vec<u8>>,
}

impl ProviderRow {
    /// Convert a row into config, opening the sealed credential with `kek`
    /// when both are present. A missing KEK or an undecryptable credential
    /// degrades to `api_key: None` with a warning rather than failing the
    /// whole config load, so one bad key cannot take down snapshot serving.
    fn into_config(self, kek: Option<&crypto::Kek>) -> Result<ProviderConfig> {
        let row = self;
        let kind = match row.kind.as_str() {
            "openai" => ProviderKind::Openai,
            "anthropic" => ProviderKind::Anthropic,
            "openai_compatible" => ProviderKind::OpenaiCompatible,
            "ollama" => ProviderKind::Ollama,
            "ollama_cloud" => ProviderKind::OllamaCloud,
            "llama_cpp" => ProviderKind::LlamaCpp,
            "openrouter" => ProviderKind::Openrouter,
            "tei" => ProviderKind::Tei,
            "azure_openai" => ProviderKind::AzureOpenai,
            "bedrock" => ProviderKind::Bedrock,
            "vertex" => ProviderKind::Vertex,
            "gemini" => ProviderKind::Gemini,
            "gemini_native" => ProviderKind::GeminiNative,
            "gemini_interactions" => ProviderKind::GeminiInteractions,
            "mistral" => ProviderKind::Mistral,
            "groq" => ProviderKind::Groq,
            "xai" => ProviderKind::Xai,
            "meta_llama_api" => ProviderKind::MetaLlamaApi,
            "cohere" => ProviderKind::Cohere,
            "perplexity" => ProviderKind::Perplexity,
            "together" => ProviderKind::Together,
            "fireworks" => ProviderKind::Fireworks,
            "databricks" => ProviderKind::Databricks,
            "aleph_alpha" => ProviderKind::AlephAlpha,
            "nebius" => ProviderKind::Nebius,
            "ovhcloud" => ProviderKind::Ovhcloud,
            "scaleway" => ProviderKind::Scaleway,
            "deepseek" => ProviderKind::Deepseek,
            "qwen" => ProviderKind::Qwen,
            "zhipu" => ProviderKind::Zhipu,
            "kimi" => ProviderKind::Kimi,
            "ernie" => ProviderKind::Ernie,
            "doubao" => ProviderKind::Doubao,
            "hunyuan" => ProviderKind::Hunyuan,
            "yi" => ProviderKind::Yi,
            "minimax" => ProviderKind::Minimax,
            "baichuan" => ProviderKind::Baichuan,
            "gigachat" => ProviderKind::Gigachat,
            "yandex_gpt" => ProviderKind::YandexGpt,
            "cloud_ru" => ProviderKind::CloudRu,
            "mts_ai" => ProviderKind::MtsAi,
            "naver" => ProviderKind::Naver,
            "upstage" => ProviderKind::Upstage,
            "rinna" => ProviderKind::Rinna,
            "rakuten" => ProviderKind::Rakuten,
            "sarvam" => ProviderKind::Sarvam,
            "krutrim" => ProviderKind::Krutrim,
            "falcon" => ProviderKind::Falcon,
            other => return Err(Error::Store(format!("unknown provider kind '{other}'"))),
        };
        let api_key = match (row.ciphertext.as_deref(), row.nonce.as_deref(), kek) {
            (Some(ciphertext), Some(nonce), Some(kek)) => match kek.decrypt(ciphertext, nonce) {
                Ok(plaintext) => Some(plaintext),
                Err(err) => {
                    tracing::warn!(provider = %row.name, error = %err,
                        "stored provider key could not be decrypted; serving provider without it");
                    None
                }
            },
            (Some(_), _, None) => {
                tracing::warn!(provider = %row.name,
                    "provider has a stored key but {} is unset; serving provider without it",
                    crypto::KEK_ENV);
                None
            }
            _ => None,
        };
        Ok(ProviderConfig {
            name: row.name,
            slug: Some(row.slug),
            kind,
            api_base: row.api_base,
            api_key,
            api_key_env: row.api_key_env,
            egress_proxy: row.egress_proxy,
            egress_proxies: serde_json::from_value(row.egress_proxies).unwrap_or_default(),
            kv_events: None,
            lmcache: None,
            ca_bundles: None,
            api_keys: Vec::new(),
            also_track_via_llm_call: false,
            llm_probe_model: None,
            status_page_url: None,
            role_profile: None,
            model_role_profiles: Default::default(),
        })
    }
}

#[derive(FromRow)]
struct RouteRow {
    id: Uuid,
    model: String,
    strategy: String,
    params: serde_json::Value,
    param_policy: serde_json::Value,
    advanced: serde_json::Value,
}

#[derive(FromRow)]
struct TargetRow {
    route_id: Uuid,
    provider_name: String,
    upstream_model: Option<String>,
    weight: i32,
}

#[derive(FromRow)]
struct ProviderGroupRow {
    id: Uuid,
    name: String,
    slug: String,
    strategy: String,
}

#[derive(FromRow)]
struct ProviderGroupMemberRow {
    group_id: Uuid,
    provider_name: String,
    upstream_model: Option<String>,
    weight: i32,
}

#[derive(FromRow)]
struct VirtualKeyRow {
    id: Uuid,
    key_hash: String,
    models: Vec<String>,
    providers: Vec<String>,
    disabled: bool,
    expires_at: Option<DateTime<Utc>>,
    cache_enabled: Option<bool>,
    project_id: Uuid,
    team_id: Uuid,
    org_id: Uuid,
    created_by: Option<Uuid>,
    business_unit_id: Option<Uuid>,
    customer_id: Option<Uuid>,
}

#[derive(FromRow)]
struct McpServerSnapshotRow {
    id: Uuid,
    org_id: Uuid,
    slug: String,
    url: String,
    transport: String,
    required_scopes: Vec<String>,
}

#[derive(FromRow)]
struct McpSessionSnapshotRow {
    id: Uuid,
    server_id: Uuid,
    user_id: Uuid,
    scopes: Vec<String>,
    expires_at: DateTime<Utc>,
    access_ciphertext: Vec<u8>,
    access_nonce: Vec<u8>,
}

#[derive(FromRow)]
struct PublishedPromptTemplateRow {
    template_id: Uuid,
    org_id: Uuid,
    slug: String,
    version: i32,
    variables: serde_json::Value,
    decorators: serde_json::Value,
}

#[derive(Clone, FromRow)]
struct PromptTemplateScopeRow {
    template_id: Uuid,
    version: i32,
    scope_type: String,
    scope_id: Uuid,
}

#[derive(FromRow)]
struct RouteScopeRow {
    route_id: Uuid,
    project_id: Uuid,
    model: String,
}

fn parse_strategy(s: &str) -> Result<BalancingStrategy> {
    Ok(match s {
        "round_robin" => BalancingStrategy::RoundRobin,
        "random" => BalancingStrategy::Random,
        "power_of_two" => BalancingStrategy::PowerOfTwo,
        "consistent_hash" => BalancingStrategy::ConsistentHash,
        "cache_aware" => BalancingStrategy::CacheAware,
        "weighted" => BalancingStrategy::Weighted,
        "pipeline" => BalancingStrategy::Pipeline,
        "cheapest" => BalancingStrategy::Cheapest,
        "fastest" => BalancingStrategy::Fastest,
        "precise_cache_aware" => BalancingStrategy::PreciseCacheAware,
        "lmcache_aware" => BalancingStrategy::LmcacheAware,
        "adaptive" => BalancingStrategy::Adaptive,
        other => {
            return Err(Error::Store(format!(
                "unknown balancing strategy '{other}'"
            )))
        }
    })
}

/// A [`ConfigStore`] backed by Postgres. `load` composes a [`GatewayConfig`]
/// from the `providers`, `routes`/`route_targets`, `model_prices`, `virtual_keys`
/// and published `prompt_templates` tables.
///
/// Virtual keys are exposed as [`rolter_core::VirtualKeyRecord`]s carrying only
/// the one-way `key_hash` plus scope identity — never the plaintext. Since the
/// gateway authenticates by peppered digest, the stored hash is sufficient to
/// verify presented keys (the control plane must hash with the same pepper).
pub struct PostgresConfigStore {
    pool: PgPool,
    /// key-encryption key for opening sealed provider credentials; read from
    /// [`crypto::KEK_ENV`] at construction. `None` serves providers without
    /// their stored keys (with a warning)
    kek: Option<crypto::Kek>,
}

impl PostgresConfigStore {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            kek: crypto::Kek::from_env(),
        }
    }

    /// Construct with an explicit KEK instead of reading the environment;
    /// mainly for tests, where mutating process-wide env vars races.
    pub fn with_kek(pool: PgPool, kek: Option<crypto::Kek>) -> Self {
        Self { pool, kek }
    }

    async fn load_providers(&self) -> Result<Vec<ProviderConfig>> {
        let rows: Vec<ProviderRow> = sqlx::query_as(
            "select p.name, p.slug, p.kind, p.api_base, p.api_key_env, p.egress_proxy, p.egress_proxies,
                    pk.ciphertext, pk.nonce
             from providers p
             left join provider_keys pk on pk.provider_id = p.id
             order by p.name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        rows.into_iter()
            .map(|row| row.into_config(self.kek.as_ref()))
            .collect()
    }

    async fn load_feature_flags(&self) -> Result<FeatureFlags> {
        sqlx::query_as(
            "select response_cache, cache_aware_routing, circuit_breaker, active_health_checks, \
                    complexity_routing, guardrails, updated_at \
             from feature_flags where id = true",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_err)
    }

    async fn load_runtime_policy(&self) -> Result<RuntimePolicy> {
        sqlx::query_as(
            "select retry_max_retries, retry_base_ms, retry_max_ms, timeout_connect_s, \
                    timeout_request_s, queue_enabled, queue_capacity, queue_workers, \
                    queue_backpressure, queue_block_ms, updated_at \
             from runtime_policy where id = true",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_err)
    }

    async fn load_compatibility_policy(&self) -> Result<CompatibilityPolicy> {
        sqlx::query_as(
            "select anthropic_version, default_max_tokens, updated_at \
             from compatibility_policy where id = true",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_err)
    }

    async fn load_adaptive_routing_policy(&self) -> Result<AdaptiveRoutingPolicy> {
        sqlx::query_as(
            "select enabled, latency_weight, cost_weight, load_weight, exploration_ratio, \
                    min_samples, updated_at \
             from adaptive_routing_policy where id = true",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_err)
    }

    async fn load_logging_settings(&self) -> Result<LoggingSettings> {
        sqlx::query_as(
            "select sample_rate, payload_capture_enabled, payload_capture_max_bytes, \
                    payload_capture_redact_fields, payload_capture_models, payload_capture_virtual_key_ids, \
                    retention_days, payload_retention_hours, updated_at \
             from logging_settings where id = true",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_err)
    }

    async fn load_routes(&self) -> Result<Vec<ModelRoute>> {
        let route_rows: Vec<RouteRow> = sqlx::query_as(
            "select id, model, strategy, params, param_policy, advanced
             from routes where enabled order by model",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;

        let target_rows: Vec<TargetRow> = sqlx::query_as(
            "select rt.route_id, p.name as provider_name, rt.upstream_model, rt.weight
             from route_targets rt
             join providers p on p.id = rt.provider_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;

        route_rows
            .into_iter()
            .map(|r| {
                let strategy = parse_strategy(&r.strategy)?;
                let targets = target_rows
                    .iter()
                    .filter(|t| t.route_id == r.id)
                    .map(|t| Target {
                        provider: t.provider_name.clone(),
                        model: t.upstream_model.clone(),
                        weight: t.weight.max(0) as u32,
                    })
                    .collect();
                // jsonb → typed; a malformed value falls back to the permissive
                // default rather than failing the whole config load
                let params = serde_json::from_value(r.params).unwrap_or_default();
                let param_policy = serde_json::from_value(r.param_policy).unwrap_or_default();
                let advanced = serde_json::from_value(r.advanced).unwrap_or_default();
                Ok(ModelRoute {
                    model: r.model,
                    strategy,
                    targets,
                    params,
                    param_policy,
                    advanced,
                    // db-backed variants land with their own store follow-up
                    variants: Default::default(),
                    // response-cache opt-in is config-only for now; a db-backed
                    // cache policy lands with its own store follow-up
                    cache: None,
                })
            })
            .collect()
    }

    /// Load provider groups and their members into `ProviderGroupConfig`
    /// (ADR-0017 addendum, ADR-0022). A member with a null `upstream_model`
    /// forwards the requested model as-is.
    async fn load_provider_groups(&self) -> Result<Vec<ProviderGroupConfig>> {
        let group_rows: Vec<ProviderGroupRow> =
            sqlx::query_as("select id, name, slug, strategy from provider_groups order by name")
                .fetch_all(&self.pool)
                .await
                .map_err(store_err)?;

        let member_rows: Vec<ProviderGroupMemberRow> = sqlx::query_as(
            "select m.group_id, p.name as provider_name, m.upstream_model, m.weight
             from provider_group_members m
             join providers p on p.id = m.provider_id
             order by m.position",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;

        group_rows
            .into_iter()
            .map(|g| {
                // validate + map the strategy the same way routes do
                let strategy = parse_strategy(&g.strategy)?;
                let members = member_rows
                    .iter()
                    .filter(|m| m.group_id == g.id)
                    .map(|m| GroupMember {
                        provider: m.provider_name.clone(),
                        model: m.upstream_model.clone(),
                        weight: m.weight.max(1) as u32,
                    })
                    .collect();
                Ok(ProviderGroupConfig {
                    name: g.name,
                    slug: Some(g.slug),
                    strategy,
                    members,
                })
            })
            .collect()
    }

    /// Load database-defined virtual keys with their resolved scope chain
    /// (project → team → org). Only the one-way `key_hash` is exposed; the
    /// gateway matches presented keys against it by peppered digest.
    async fn load_virtual_keys(&self) -> Result<Vec<VirtualKeyRecord>> {
        let rows: Vec<VirtualKeyRow> = sqlx::query_as(
            "select vk.id, vk.key_hash, vk.models, vk.providers, vk.disabled, vk.expires_at, \
                    vk.cache_enabled, vk.project_id, p.team_id, t.org_id, vk.created_by, \
                    vk.business_unit_id, vk.customer_id \
             from virtual_keys vk \
             join projects p on p.id = vk.project_id \
             join teams t on t.id = p.team_id \
             order by vk.created_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        Ok(rows
            .into_iter()
            .map(|r| VirtualKeyRecord {
                key_hash: r.key_hash,
                id: r.id.to_string(),
                org_id: r.org_id.to_string(),
                team_id: r.team_id.to_string(),
                project_id: r.project_id.to_string(),
                user_id: r.created_by.map(|id| id.to_string()).unwrap_or_default(),
                models: r.models,
                providers: r.providers,
                disabled: r.disabled,
                expires_at: r.expires_at,
                cache: r.cache_enabled,
                business_unit_id: r
                    .business_unit_id
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                customer_id: r.customer_id.map(|id| id.to_string()).unwrap_or_default(),
            })
            .collect())
    }

    async fn load_mcp_servers(&self) -> Result<Vec<McpServerConfig>> {
        let rows: Vec<McpServerSnapshotRow> = sqlx::query_as(
            "select id, org_id, slug, url, transport, required_scopes \
             from mcp_servers where enabled order by org_id, slug",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        Ok(rows
            .into_iter()
            .map(|row| McpServerConfig {
                id: row.id.to_string(),
                org_id: row.org_id.to_string(),
                slug: row.slug,
                url: row.url,
                transport: row.transport,
                required_scopes: row.required_scopes,
            })
            .collect())
    }

    async fn load_mcp_oauth_sessions(&self) -> Result<Vec<McpOAuthSessionConfig>> {
        let rows: Vec<McpSessionSnapshotRow> = sqlx::query_as(
            "select distinct on (g.server_id, g.user_id) \
                    s.id, g.server_id, g.user_id, s.scopes, s.expires_at, \
                    s.access_ciphertext, s.access_nonce \
             from mcp_oauth_sessions s \
             join mcp_oauth_grants g on g.id = s.grant_id \
             join mcp_servers srv on srv.id = g.server_id \
             where s.revoked_at is null and g.revoked_at is null \
               and s.expires_at > now() and s.scopes <@ g.scopes \
               and srv.required_scopes <@ s.scopes \
             order by g.server_id, g.user_id, s.created_at desc",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        let Some(kek) = self.kek.as_ref() else {
            if !rows.is_empty() {
                tracing::warn!(
                    "MCP OAuth sessions exist but {} is unset; omitting them from the snapshot",
                    crypto::KEK_ENV
                );
            }
            return Ok(Vec::new());
        };
        Ok(rows
            .into_iter()
            .filter_map(
                |row| match kek.decrypt(&row.access_ciphertext, &row.access_nonce) {
                    Ok(access_token) => Some(McpOAuthSessionConfig {
                        id: row.id.to_string(),
                        server_id: row.server_id.to_string(),
                        user_id: row.user_id.to_string(),
                        scopes: row.scopes,
                        expires_at: row.expires_at,
                        access_token,
                    }),
                    Err(error) => {
                        tracing::warn!(
                            session_id = %row.id,
                            error = %error,
                            "stored MCP access token could not be decrypted; omitting the session"
                        );
                        None
                    }
                },
            )
            .collect())
    }

    async fn load_model_prices(&self) -> Result<Vec<ModelPriceConfig>> {
        let rows: Vec<ModelPrice> = sqlx::query_as(
            "select id, model, \
                    input_per_mtok::text as input_per_mtok, \
                    output_per_mtok::text as output_per_mtok, \
                    cached_input_per_mtok::text as cached_input_per_mtok, \
                    currency, created_at \
             from model_prices order by model",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        Ok(rows
            .into_iter()
            .map(|r| ModelPriceConfig {
                model: r.model,
                // decimals are stored as text; a malformed value prices at zero
                input_per_mtok: r.input_per_mtok.parse().unwrap_or(0.0),
                output_per_mtok: r.output_per_mtok.parse().unwrap_or(0.0),
                cached_input_per_mtok: r.cached_input_per_mtok.and_then(|v| v.parse().ok()),
                // the column has always existed and the dashboard has always
                // written it; it just never reached the config (#650), so every
                // non-USD price was charged as if it were USD
                currency: r.currency,
            })
            .collect())
    }

    async fn load_budgets(&self) -> Result<Vec<BudgetConfig>> {
        let rows: Vec<Budget> = sqlx::query_as(
            // limit_usd is numeric(12,4); decode it as text (Budget.limit_usd is a
            // String) or sqlx errors and the whole snapshot 500s — freezing every
            // polling gateway on its last config the moment any budget exists
            "select id, scope_type, scope_id, limit_usd::text as limit_usd, period, created_at
             from budgets order by created_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let scope = match r.scope_type.as_str() {
                    "org" => BudgetScope::Org,
                    "team" => BudgetScope::Team,
                    "project" => BudgetScope::Project,
                    "virtual_key" => BudgetScope::Key,
                    "business_unit" => BudgetScope::BusinessUnit,
                    "customer" => BudgetScope::Customer,
                    // unknown scope: skip rather than mis-enforce
                    _ => return None,
                };
                Some(BudgetConfig {
                    scope,
                    id: r.scope_id.to_string(),
                    // decimal stored as text; a malformed value disables the cap
                    limit_usd: r.limit_usd.parse().unwrap_or(f64::INFINITY),
                    period: parse_period(&r.period),
                })
            })
            .collect())
    }

    async fn load_rate_limits(&self) -> Result<Vec<RateLimitConfig>> {
        let rows: Vec<RateLimit> = sqlx::query_as(
            "select id, scope_type, scope_id, rpm, tpm, created_at
             from rate_limits order by created_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let scope = match r.scope_type.as_str() {
                    "org" => BudgetScope::Org,
                    "team" => BudgetScope::Team,
                    "project" => BudgetScope::Project,
                    "virtual_key" => BudgetScope::Key,
                    "business_unit" => BudgetScope::BusinessUnit,
                    "customer" => BudgetScope::Customer,
                    // unknown scope: skip rather than mis-enforce
                    _ => return None,
                };
                Some(RateLimitConfig {
                    scope,
                    id: r.scope_id.to_string(),
                    // non-positive caps are meaningless; treat them as unset
                    rpm: r.rpm.filter(|v| *v > 0).map(|v| v as u32),
                    tpm: r.tpm.filter(|v| *v > 0).map(|v| v as u32),
                })
            })
            .collect())
    }

    /// Load published prompt templates with tenant-aware activation bindings.
    async fn load_prompt_templates(&self) -> Result<PromptTemplatesConfig> {
        let templates: Vec<PublishedPromptTemplateRow> = sqlx::query_as(
            "select t.id as template_id, t.org_id, t.slug, v.version, v.variables, v.decorators
             from prompt_templates t
             join prompt_template_versions v
               on v.template_id = t.id
              and v.version = t.published_version
             order by t.slug",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        if templates.is_empty() {
            return Ok(PromptTemplatesConfig::default());
        }

        let scopes: Vec<PromptTemplateScopeRow> = sqlx::query_as(
            "select s.template_id, s.version, s.scope_type, s.scope_id
             from prompt_template_scopes s
             join prompt_templates t on t.id = s.template_id
             where s.version = t.published_version
             order by s.template_id, s.version, s.scope_type, s.scope_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;

        let route_rows: Vec<RouteScopeRow> = sqlx::query_as(
            "select id as route_id, project_id, model
             from routes
             where enabled",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_err)?;
        let route_identity: HashMap<Uuid, (Uuid, String)> = route_rows
            .iter()
            .map(|route| (route.route_id, (route.project_id, route.model.clone())))
            .collect();

        let mut scopes_by_template: HashMap<(Uuid, i32), Vec<PromptTemplateScopeRow>> =
            HashMap::new();
        for scope in scopes {
            scopes_by_template
                .entry((scope.template_id, scope.version))
                .or_default()
                .push(scope);
        }

        let mut compiled = Vec::new();
        for row in templates {
            if row.version <= 0 {
                tracing::warn!(
                    template_id = %row.template_id,
                    version = row.version,
                    "skipping prompt template with non-positive published version"
                );
                continue;
            }
            let variables: Vec<TemplateVariable> = match serde_json::from_value(row.variables) {
                Ok(variables) => variables,
                Err(err) => {
                    tracing::warn!(
                        template_id = %row.template_id,
                        error = %err,
                        "skipping prompt template with invalid variables json"
                    );
                    continue;
                }
            };
            let decorators: Vec<Decorator> = match serde_json::from_value(row.decorators) {
                Ok(decorators) => decorators,
                Err(err) => {
                    tracing::warn!(
                        template_id = %row.template_id,
                        error = %err,
                        "skipping prompt template with invalid decorators json"
                    );
                    continue;
                }
            };
            if decorators.is_empty() {
                tracing::warn!(
                    template_id = %row.template_id,
                    "skipping prompt template with empty decorators"
                );
                continue;
            }

            let scope_rows = scopes_by_template
                .get(&(row.template_id, row.version))
                .cloned()
                .unwrap_or_default();
            if scope_rows.is_empty() {
                tracing::warn!(
                    template_id = %row.template_id,
                    "skipping published prompt template without scope bindings"
                );
                continue;
            }

            let mut activations = Vec::with_capacity(scope_rows.len());
            for scope in scope_rows {
                match scope.scope_type.as_str() {
                    "org" => activations.push(PromptTemplateActivationScope::Org {
                        id: scope.scope_id.to_string(),
                    }),
                    "project" => activations.push(PromptTemplateActivationScope::Project {
                        id: scope.scope_id.to_string(),
                    }),
                    "route" => {
                        if let Some((project_id, model)) = route_identity.get(&scope.scope_id) {
                            activations.push(PromptTemplateActivationScope::Route {
                                project_id: project_id.to_string(),
                                model: model.clone(),
                            });
                        }
                    }
                    "virtual_key" => activations.push(PromptTemplateActivationScope::VirtualKey {
                        id: scope.scope_id.to_string(),
                    }),
                    other => {
                        tracing::warn!(
                            template_id = %row.template_id,
                            scope_type = other,
                            "unknown prompt-template scope type skipped"
                        );
                    }
                }
            }

            if activations.is_empty() {
                tracing::warn!(
                    template_id = %row.template_id,
                    "skipping prompt template because no resolvable route scopes were found"
                );
                continue;
            }

            compiled.push(PromptTemplate {
                id: format!("{}:{}", row.org_id, row.slug),
                version: row.version as u32,
                routes: Vec::new(),
                scopes: activations,
                variables,
                decorators,
            });
        }
        compiled.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.version.cmp(&right.version))
        });

        Ok(PromptTemplatesConfig {
            enabled: !compiled.is_empty(),
            templates: compiled,
        })
    }
}

/// Map the free-text `budgets.period` column to a [`BudgetPeriod`]. Accepts both
/// the human names and the legacy duration shorthands (`1d`, `30d`), defaulting
/// to monthly for anything unrecognized.
fn parse_period(period: &str) -> BudgetPeriod {
    match period.trim().to_ascii_lowercase().as_str() {
        "daily" | "1d" | "24h" => BudgetPeriod::Daily,
        "total" | "lifetime" | "all" => BudgetPeriod::Total,
        _ => BudgetPeriod::Monthly,
    }
}

/// Read the current global config version. Bumping happens in the database
/// itself: migration 0003 installs triggers that increment the version
/// atomically with any write to providers/routes/route_targets/virtual_keys.
pub async fn current_version(pool: &PgPool) -> Result<i64> {
    sqlx::query_scalar("select version from config_version where id = 1")
        .fetch_one(pool)
        .await
        .map_err(store_err)
}

#[async_trait]
impl ConfigStore for PostgresConfigStore {
    async fn load(&self) -> Result<GatewayConfig> {
        let flags = self.load_feature_flags().await?;
        let runtime_policy = self.load_runtime_policy().await?;
        let logging = self.load_logging_settings().await?;
        let compatibility = self.load_compatibility_policy().await?;
        let adaptive = self.load_adaptive_routing_policy().await?;
        let providers = self.load_providers().await?;
        let routes = self.load_routes().await?;
        let provider_groups = self.load_provider_groups().await?;
        let model_prices = self.load_model_prices().await?;
        let db_virtual_keys = self.load_virtual_keys().await?;
        let mcp_servers = self.load_mcp_servers().await?;
        let mcp_oauth_sessions = self.load_mcp_oauth_sessions().await?;
        let budgets = self.load_budgets().await?;
        let rate_limits = self.load_rate_limits().await?;
        let prompt_templates = self.load_prompt_templates().await?;
        let mut config = GatewayConfig {
            providers,
            routes,
            provider_groups,
            model_prices,
            db_virtual_keys,
            mcp_servers,
            mcp_oauth_sessions,
            budgets,
            rate_limits,
            prompt_templates,
            feature_flags: FeatureFlagsConfig {
                response_cache: flags.response_cache,
                cache_aware_routing: flags.cache_aware_routing,
                circuit_breaker: flags.circuit_breaker,
                active_health_checks: flags.active_health_checks,
                complexity_routing: flags.complexity_routing,
                guardrails: flags.guardrails,
            },
            ..GatewayConfig::default()
        };
        config.apply_feature_flags();
        config.logging.sample_rate = logging.sample_rate;
        config.logging.payload_capture.enabled = logging.payload_capture_enabled;
        config.logging.payload_capture.max_bytes =
            logging.payload_capture_max_bytes.max(0) as usize;
        config.logging.payload_capture.redact_fields = logging.payload_capture_redact_fields;
        config.logging.payload_capture.models = logging.payload_capture_models;
        config.logging.payload_capture.virtual_key_ids = logging.payload_capture_virtual_key_ids;
        config.retry.max_retries = runtime_policy.retry_max_retries.max(0) as u32;
        config.retry.base_backoff_ms = runtime_policy.retry_base_ms.max(0) as u64;
        config.retry.max_backoff_ms = runtime_policy.retry_max_ms.max(0) as u64;
        config.timeouts.connect_secs = runtime_policy.timeout_connect_s.max(0) as u64;
        config.timeouts.request_secs = runtime_policy.timeout_request_s.max(0) as u64;
        config.queue.enabled = runtime_policy.queue_enabled;
        config.queue.capacity = runtime_policy.queue_capacity.max(1) as usize;
        config.queue.workers = runtime_policy.queue_workers.max(1) as usize;
        config.queue.backpressure = match runtime_policy.queue_backpressure.as_str() {
            "drop" => BackpressurePolicy::Drop,
            "block" => BackpressurePolicy::Block,
            _ => BackpressurePolicy::Error,
        };
        config.queue.block_timeout_ms = runtime_policy.queue_block_ms.max(0) as u64;
        config.compatibility.anthropic_version = compatibility.anthropic_version;
        config.compatibility.default_max_tokens = compatibility.default_max_tokens.max(1) as u32;
        config.adaptive_routing = rolter_core::AdaptiveRoutingConfig {
            enabled: adaptive.enabled,
            latency_weight: adaptive.latency_weight,
            cost_weight: adaptive.cost_weight,
            load_weight: adaptive.load_weight,
            exploration_ratio: adaptive.exploration_ratio,
            min_samples: adaptive.min_samples.max(0) as u32,
        }
        .sanitized();
        Ok(config)
    }

    async fn save(&self, _config: GatewayConfig) -> Result<()> {
        Err(Error::Store(
            "PostgresConfigStore is read-only; use the control-plane CRUD API to mutate providers/routes"
                .into(),
        ))
    }

    async fn current_version(&self) -> Result<i64> {
        sqlx::query_scalar("select version from config_version where id = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(store_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_url() -> Option<String> {
        std::env::var("ROLTER_TEST_DATABASE_URL").ok()
    }

    async fn fresh_pool() -> PgPool {
        let url = database_url().expect("ROLTER_TEST_DATABASE_URL not set; skipping");
        super::test_support::fresh_scoped_pool(&url).await
    }

    #[tokio::test]
    async fn triggers_bump_version_atomically_with_writes() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;
        let v0 = current_version(&pool).await.unwrap();

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // orgs don't feed the gateway snapshot: no bump
        assert_eq!(current_version(&pool).await.unwrap(), v0);

        sqlx::query(
            "insert into providers (org_id, name, slug, kind, api_base)
             values ($1, 'openai', 'openai', 'openai', 'https://api.openai.com')",
        )
        .bind(org_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 1);

        // a rolled-back write must not bump the version
        let mut txn = pool.begin().await.unwrap();
        sqlx::query(
            "insert into providers (org_id, name, slug, kind, api_base)
             values ($1, 'ghost', 'ghost', 'openai', 'https://ghost.example.com')",
        )
        .bind(org_id)
        .execute(&mut *txn)
        .await
        .unwrap();
        txn.rollback().await.unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 1);

        sqlx::query("delete from providers where name = 'openai'")
            .execute(&pool)
            .await
            .unwrap();
        // the provider delete cascades to provider_keys, whose statement trigger
        // bumps the version even when no key rows exist
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 3);
    }

    #[tokio::test]
    async fn prompt_template_writes_bump_version() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;
        let v0 = current_version(&pool).await.unwrap();

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let template_id: Uuid = sqlx::query_scalar(
            "insert into prompt_templates (org_id, name, slug, description)
             values ($1, 'support baseline', 'support-baseline', 'desc')
             returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 1);

        sqlx::query(
            "insert into prompt_template_versions (template_id, version, variables, decorators)
             values
               ($1, 1, '[]'::jsonb, '[{\"role\":\"system\",\"position\":\"prepend\",\"content\":\"v1\"}]'::jsonb)",
        )
        .bind(template_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 2);

        sqlx::query(
            "insert into prompt_template_scopes (
                 template_id, version, scope_type, scope_id, org_id
             )
             values ($1, 1, 'org', $2, $2)",
        )
        .bind(template_id)
        .bind(org_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 3);

        sqlx::query(
            "update prompt_templates
             set published_version = 1
             where id = $1",
        )
        .bind(template_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 4);
    }

    #[tokio::test]
    async fn skills_writes_bump_version() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;
        let v0 = current_version(&pool).await.unwrap();

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let skill_id: Uuid = sqlx::query_scalar(
            "insert into skills (org_id, name, slug, description)
             values ($1, 'classification baseline', 'classification-baseline', 'desc')
             returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 1);

        sqlx::query(
            "insert into skill_versions (skill_id, version, content, metadata)
             values ($1, 1, 'alpha', '{}'::jsonb)",
        )
        .bind(skill_id)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 2);

        sqlx::query("update skills set published_version = 1 where id = $1")
            .bind(skill_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(current_version(&pool).await.unwrap(), v0 + 3);
    }

    #[tokio::test]
    async fn loads_providers_and_routes_from_db() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let team_id: Uuid =
            sqlx::query_scalar("insert into teams (org_id, name) values ($1, 'core') returning id")
                .bind(org_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let project_id: Uuid = sqlx::query_scalar(
            "insert into projects (team_id, name) values ($1, 'default') returning id",
        )
        .bind(team_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let provider_id: Uuid = sqlx::query_scalar(
            "insert into providers (org_id, name, slug, kind, api_base, api_key_env)
             values ($1, 'openai', 'openai', 'openai', 'https://api.openai.com', 'OPENAI_API_KEY')
             returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let route_id: Uuid = sqlx::query_scalar(
            "insert into routes (project_id, model, strategy) values ($1, 'gpt-4o', 'power_of_two') returning id",
        )
        .bind(project_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into route_targets (route_id, provider_id, upstream_model, weight)
             values ($1, $2, 'gpt-4o-2024-08-06', 2)",
        )
        .bind(route_id)
        .bind(provider_id)
        .execute(&pool)
        .await
        .unwrap();

        let store = PostgresConfigStore::new(pool);
        let config = store.load().await.unwrap();

        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].name, "openai");
        assert_eq!(config.providers[0].kind, ProviderKind::Openai);
        assert_eq!(
            config.providers[0].api_key_env.as_deref(),
            Some("OPENAI_API_KEY")
        );

        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].model, "gpt-4o");
        assert_eq!(config.routes[0].strategy, BalancingStrategy::PowerOfTwo);
        assert_eq!(config.routes[0].targets.len(), 1);
        assert_eq!(config.routes[0].targets[0].provider, "openai");
        assert_eq!(
            config.routes[0].targets[0].model.as_deref(),
            Some("gpt-4o-2024-08-06")
        );
        assert_eq!(config.routes[0].targets[0].weight, 2);
    }

    #[tokio::test]
    async fn feature_flags_gate_supported_snapshot_subsystems() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let team_id: Uuid =
            sqlx::query_scalar("insert into teams (org_id, name) values ($1, 'core') returning id")
                .bind(org_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let project_id: Uuid = sqlx::query_scalar(
            "insert into projects (team_id, name) values ($1, 'default') returning id",
        )
        .bind(team_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let provider_id: Uuid = sqlx::query_scalar(
            "insert into providers (org_id, name, slug, kind, api_base, api_key_env)
             values ($1, 'openai', 'openai', 'openai', 'https://api.openai.com', 'OPENAI_API_KEY')
             returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let route_id: Uuid = sqlx::query_scalar(
            "insert into routes (project_id, model, strategy, params)
             values ($1, 'gpt-4o', 'cache_aware', $2) returning id",
        )
        .bind(project_id)
        .bind(serde_json::json!({
            "_rolter_complexity": {
                "tiers": [{"name":"simple","max_input_bytes":256,"route":"gpt-4o"}]
            }
        }))
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into route_targets (route_id, provider_id, upstream_model, weight)
             values ($1, $2, 'gpt-4o-2024-08-06', 1)",
        )
        .bind(route_id)
        .bind(provider_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "update feature_flags set
                response_cache = false,
                cache_aware_routing = false,
                circuit_breaker = false,
                active_health_checks = false,
                complexity_routing = false,
                guardrails = false
             where id = true",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = PostgresConfigStore::new(pool);
        let config = store.load().await.unwrap();
        assert!(!config.cache.enabled);
        assert!(!config.breaker.enabled);
        assert!(!config.health.enabled);
        assert!(!config.guardrails.enabled);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].strategy, BalancingStrategy::PowerOfTwo);
        assert!(!config.routes[0].params.contains_key("_rolter_complexity"));
    }

    #[tokio::test]
    async fn logging_settings_project_into_snapshot_logging_policy() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;
        sqlx::query(
            "update logging_settings set
                sample_rate = 0.4,
                payload_capture_enabled = true,
                payload_capture_max_bytes = 2048,
                payload_capture_redact_fields = array['token','secret'],
                payload_capture_models = array['gpt-4o'],
                payload_capture_virtual_key_ids = array['vk-1']
             where id = true",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = PostgresConfigStore::new(pool);
        let config = store.load().await.unwrap();
        assert_eq!(config.logging.sample_rate, 0.4);
        assert!(config.logging.payload_capture.enabled);
        assert_eq!(config.logging.payload_capture.max_bytes, 2048);
        assert_eq!(
            config.logging.payload_capture.redact_fields,
            vec!["token".to_string(), "secret".to_string()]
        );
        assert_eq!(
            config.logging.payload_capture.models,
            vec!["gpt-4o".to_string()]
        );
        assert_eq!(
            config.logging.payload_capture.virtual_key_ids,
            vec!["vk-1".to_string()]
        );
    }

    #[tokio::test]
    async fn runtime_policy_projects_into_snapshot_runtime_controls() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;
        sqlx::query(
            "update runtime_policy set
                retry_max_retries = 5,
                retry_base_ms = 250,
                retry_max_ms = 4000,
                timeout_connect_s = 15,
                timeout_request_s = 120,
                queue_enabled = false,
                queue_capacity = 512,
                queue_workers = 32,
                queue_backpressure = 'drop',
                queue_block_ms = 750
             where id = true",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = PostgresConfigStore::new(pool);
        let config = store.load().await.unwrap();
        assert_eq!(config.retry.max_retries, 5);
        assert_eq!(config.retry.base_backoff_ms, 250);
        assert_eq!(config.retry.max_backoff_ms, 4000);
        assert_eq!(config.timeouts.connect_secs, 15);
        assert_eq!(config.timeouts.request_secs, 120);
        assert!(!config.queue.enabled);
        assert_eq!(config.queue.capacity, 512);
        assert_eq!(config.queue.workers, 32);
        assert_eq!(config.queue.backpressure, BackpressurePolicy::Drop);
        assert_eq!(config.queue.block_timeout_ms, 750);
    }

    // regression: the snapshot query must cast numeric price columns to text,
    // otherwise sqlx fails decoding into String and GET /api/v1/models 500s
    // as soon as any model_prices row exists
    #[tokio::test]
    async fn loads_model_prices_from_db() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        sqlx::query(
            "insert into model_prices (model, input_per_mtok, output_per_mtok, cached_input_per_mtok, currency)
             values ('gpt-4o', 3, 15, 1.5, 'USD'), ('gpt-4o-mini', 0.15, 0.6, null, 'USD')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = PostgresConfigStore::new(pool);
        let config = store.load().await.unwrap();

        assert_eq!(config.model_prices.len(), 2);
        assert_eq!(config.model_prices[0].model, "gpt-4o");
        assert_eq!(config.model_prices[0].input_per_mtok, 3.0);
        assert_eq!(config.model_prices[0].output_per_mtok, 15.0);
        assert_eq!(config.model_prices[0].cached_input_per_mtok, Some(1.5));
        assert_eq!(config.model_prices[1].model, "gpt-4o-mini");
        assert_eq!(config.model_prices[1].input_per_mtok, 0.15);
        assert_eq!(config.model_prices[1].cached_input_per_mtok, None);
    }

    #[tokio::test]
    async fn loads_published_prompt_templates_from_db() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let team_id: Uuid =
            sqlx::query_scalar("insert into teams (org_id, name) values ($1, 'core') returning id")
                .bind(org_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let project_id: Uuid = sqlx::query_scalar(
            "insert into projects (team_id, name) values ($1, 'default') returning id",
        )
        .bind(team_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let route_a: Uuid = sqlx::query_scalar(
            "insert into routes (project_id, model, strategy)
             values ($1, 'gpt-4o-mini', 'round_robin')
             returning id",
        )
        .bind(project_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into routes (project_id, model, strategy)
             values ($1, 'gpt-4o', 'round_robin')",
        )
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();

        let template_id: Uuid = sqlx::query_scalar(
            "insert into prompt_templates (org_id, name, slug, description)
             values ($1, 'support baseline', 'support-baseline', 'desc')
             returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into prompt_template_versions (template_id, version, variables, decorators)
             values
                ($1, 1, '[]'::jsonb, '[]'::jsonb),
                ($1, 2, '[{\"name\":\"tone\",\"required\":true}]'::jsonb,
                        '[{\"role\":\"system\",\"position\":\"prepend\",\"content\":\"tone={{ tone }}\"}]'::jsonb)",
        )
        .bind(template_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into prompt_template_scopes (
                 template_id, version, scope_type, scope_id, project_id, route_id
             )
             values
                ($1, 2, 'project', $2, $2, null),
                ($1, 2, 'route', $3, null, $3)",
        )
        .bind(template_id)
        .bind(project_id)
        .bind(route_a)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("update prompt_templates set published_version = 2 where id = $1")
            .bind(template_id)
            .execute(&pool)
            .await
            .unwrap();

        let global_template_id: Uuid = sqlx::query_scalar(
            "insert into prompt_templates (org_id, name, slug, description)
             values ($1, 'global baseline', 'global-baseline', 'desc')
             returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into prompt_template_versions (template_id, version, variables, decorators)
             values
                ($1, 1, '[]'::jsonb, '[{\"role\":\"system\",\"position\":\"prepend\",\"content\":\"always on\"}]'::jsonb)",
        )
        .bind(global_template_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into prompt_template_scopes (
                 template_id, version, scope_type, scope_id, org_id
             )
             values ($1, 1, 'org', $2, $2)",
        )
        .bind(global_template_id)
        .bind(org_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("update prompt_templates set published_version = 1 where id = $1")
            .bind(global_template_id)
            .execute(&pool)
            .await
            .unwrap();

        let store = PostgresConfigStore::new(pool);
        let config = store.load().await.unwrap();
        assert!(config.prompt_templates.enabled);
        assert_eq!(config.prompt_templates.templates.len(), 2);

        let global = config
            .prompt_templates
            .templates
            .iter()
            .find(|template| template.id == format!("{org_id}:global-baseline"))
            .unwrap();
        assert_eq!(global.version, 1);
        assert!(global.routes.is_empty());
        assert_eq!(
            global.scopes,
            vec![PromptTemplateActivationScope::Org {
                id: org_id.to_string()
            }]
        );

        let scoped = config
            .prompt_templates
            .templates
            .iter()
            .find(|template| template.id == format!("{org_id}:support-baseline"))
            .unwrap();
        assert_eq!(scoped.version, 2);
        assert_eq!(
            scoped.scopes,
            vec![
                PromptTemplateActivationScope::Project {
                    id: project_id.to_string()
                },
                PromptTemplateActivationScope::Route {
                    project_id: project_id.to_string(),
                    model: "gpt-4o-mini".to_string()
                }
            ]
        );
        assert_eq!(scoped.variables.len(), 1);
        assert_eq!(scoped.decorators.len(), 1);
    }

    // regression: like model_prices, the snapshot query must cast budgets.limit_usd
    // (numeric) to text — Budget.limit_usd is a String — or store.load() errors and
    // every polling gateway freezes on its last config the moment any budget exists
    #[tokio::test]
    async fn loads_budgets_from_db() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into budgets (scope_type, scope_id, limit_usd, period)
             values ('org', $1, 100.5, '30d')",
        )
        .bind(org_id)
        .execute(&pool)
        .await
        .unwrap();

        let store = PostgresConfigStore::new(pool);
        // must not error decoding the numeric column into Budget.limit_usd (String)
        let config = store.load().await.unwrap();

        assert_eq!(config.budgets.len(), 1);
        assert_eq!(config.budgets[0].limit_usd, 100.5);
    }

    // governance dimensions carry their own caps, so a business-unit budget and
    // a customer rate limit must survive the snapshot projection (#539)
    #[tokio::test]
    async fn loads_governance_scoped_budgets_and_limits() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let unit_id: Uuid = sqlx::query_scalar(
            "insert into business_units (org_id, name, slug) values ($1, 'Payments', 'payments') returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let customer_id: Uuid = sqlx::query_scalar(
            "insert into customers (org_id, name, slug) values ($1, 'Acme EU', 'acme-eu') returning id",
        )
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into budgets (scope_type, scope_id, limit_usd, period)
             values ('business_unit', $1, 250.0, '30d')",
        )
        .bind(unit_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into rate_limits (scope_type, scope_id, rpm, tpm)
             values ('customer', $1, 60, 90000)",
        )
        .bind(customer_id)
        .execute(&pool)
        .await
        .unwrap();

        let config = PostgresConfigStore::new(pool).load().await.unwrap();

        let budget = config
            .budgets
            .iter()
            .find(|b| b.scope == BudgetScope::BusinessUnit)
            .expect("business-unit budget in snapshot");
        assert_eq!(budget.id, unit_id.to_string());
        assert_eq!(budget.limit_usd, 250.0);

        let limit = config
            .rate_limits
            .iter()
            .find(|l| l.scope == BudgetScope::Customer)
            .expect("customer rate limit in snapshot");
        assert_eq!(limit.id, customer_id.to_string());
        assert_eq!(limit.rpm, Some(60));
    }

    // the adaptive-routing kill switch and blend weights are control-plane
    // owned, so an operator change must reach the polling gateway (#544)
    #[tokio::test]
    async fn adaptive_routing_policy_projects_into_snapshot() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        // the shipped defaults leave adaptive routing off
        let config = PostgresConfigStore::new(pool.clone()).load().await.unwrap();
        assert!(!config.adaptive_routing.enabled);
        assert_eq!(config.adaptive_routing.min_samples, 50);

        sqlx::query(
            "update adaptive_routing_policy set enabled = true, latency_weight = 2.0, \
                    cost_weight = 0, load_weight = 0.5, exploration_ratio = 0.1, \
                    min_samples = 10 where id = true",
        )
        .execute(&pool)
        .await
        .unwrap();

        let config = PostgresConfigStore::new(pool).load().await.unwrap();
        assert!(config.adaptive_routing.enabled);
        assert_eq!(config.adaptive_routing.latency_weight, 2.0);
        assert_eq!(config.adaptive_routing.cost_weight, 0.0);
        assert_eq!(config.adaptive_routing.exploration_ratio, 0.1);
        assert_eq!(config.adaptive_routing.min_samples, 10);
    }

    // compiled-in translation constants are control-plane owned now, so the
    // persisted policy must reach the snapshot the gateway polls (#546)
    #[tokio::test]
    async fn compatibility_policy_projects_into_snapshot() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;

        // defaults preserve the previous hardcoded behavior
        let config = PostgresConfigStore::new(pool.clone()).load().await.unwrap();
        assert_eq!(config.compatibility.anthropic_version, "2023-06-01");
        assert_eq!(config.compatibility.default_max_tokens, 1024);

        sqlx::query(
            "update compatibility_policy set anthropic_version = '2024-10-22', \
                    default_max_tokens = 4096 where id = true",
        )
        .execute(&pool)
        .await
        .unwrap();

        let config = PostgresConfigStore::new(pool).load().await.unwrap();
        assert_eq!(config.compatibility.anthropic_version, "2024-10-22");
        assert_eq!(config.compatibility.default_max_tokens, 4096);
    }

    #[tokio::test]
    async fn save_is_read_only() {
        let Some(_) = database_url() else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let pool = fresh_pool().await;
        let store = PostgresConfigStore::new(pool);
        assert!(store.save(GatewayConfig::default()).await.is_err());
    }
}
