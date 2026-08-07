//! rolter-control: the control plane.
//!
//! Hosts the management API used by the dashboard and serves the built SPA as
//! static assets. The MVP exposes health, a config read endpoint backed by an
//! in-memory store, and the role catalog; CRUD, RBAC enforcement, Postgres
//! persistence and Redis change publication are added in later phases.
//!
//! The binary is a thin wrapper over [`run`]; the unified `rolter` launcher
//! reuses the same entrypoint as its `control` subcommand.

#[cfg(feature = "postgres")]
mod access_control;
#[cfg(feature = "postgres")]
mod adaptive_policy;
#[cfg(feature = "postgres")]
mod adaptive_telemetry;
#[cfg(feature = "postgres")]
mod alerting;
mod analytics;
#[cfg(feature = "postgres")]
mod auth;
#[cfg(feature = "postgres")]
mod auth_policy;
#[cfg(feature = "postgres")]
mod client_settings;
#[cfg(feature = "postgres")]
mod cluster;
#[cfg(feature = "postgres")]
mod compatibility_policy;
#[cfg(feature = "postgres")]
mod connectors;
mod cors;
#[cfg(feature = "postgres")]
mod crud;
#[cfg(feature = "postgres")]
mod feature_flags;
#[cfg(feature = "postgres")]
mod guardrails;
mod health;
#[cfg(feature = "postgres")]
mod invitations;
#[cfg(feature = "postgres")]
mod logging_settings;
#[cfg(feature = "postgres")]
mod mcp_logs;
#[cfg(feature = "postgres")]
mod mcp_oauth;
#[cfg(feature = "postgres")]
mod mcp_oauth_flow;
#[cfg(feature = "postgres")]
mod me;
#[cfg(feature = "postgres")]
mod model_defaults;
#[cfg(feature = "postgres")]
mod plugins;
mod proxy;
#[cfg(feature = "postgres")]
mod rbac;
#[cfg(feature = "postgres")]
mod rbac_matrix;
#[cfg(feature = "postgres")]
mod runtime_policy;
#[cfg(feature = "postgres")]
mod scim;
#[cfg(feature = "postgres")]
mod scim_groups;
#[cfg(feature = "postgres")]
mod security;
#[cfg(feature = "postgres")]
pub mod seed;
#[cfg(feature = "postgres")]
mod sso;
mod ui_config;
#[cfg(feature = "postgres")]
mod ui_events;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::ServeDir;

use rolter_auth::Role;
use rolter_core::GatewayConfig;
#[cfg(feature = "postgres")]
use rolter_store::MergedConfigStore;
use rolter_store::{ConfigStore, InMemoryConfigStore};

#[derive(Parser, Debug)]
#[command(name = "rolter-control", version, about = "rolter control plane")]
pub struct Args {
    #[arg(long, env = "ROLTER_CONTROL_HOST", default_value = "0.0.0.0")]
    pub host: String,
    #[arg(long, env = "ROLTER_CONTROL_PORT", default_value_t = 4001)]
    pub port: u16,
    /// directory holding the built UI (index.html + assets)
    #[arg(long, env = "ROLTER_UI_DIR", default_value = "ui/dist")]
    pub ui_dir: PathBuf,
    /// OTLP/HTTP traces endpoint for the *dashboard's* browser tracing, e.g.
    /// `http://localhost:4318/v1/traces`. Injected into the served HTML as
    /// `window.__ROLTER_CONFIG__`; unset (the default) leaves browser tracing
    /// off, loading no SDK and changing no headers. This is deliberately not
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`: that one is the backend's own gRPC
    /// exporter, while a browser needs an HTTP endpoint reachable from the
    /// user's machine rather than from inside the cluster
    #[arg(long, env = "ROLTER_UI_OTEL_ENDPOINT")]
    pub ui_otel_endpoint: Option<String>,
    /// `service.name` reported by the dashboard's spans; the dashboard falls
    /// back to `rolter-ui` when this is unset
    #[arg(long, env = "ROLTER_UI_OTEL_SERVICE_NAME")]
    pub ui_otel_service_name: Option<String>,
    /// base URL of the rolter-gateway data plane; the dashboard Playground's
    /// `/gw/*` calls are reverse-proxied here (see `crate::proxy`)
    #[arg(
        long,
        env = "ROLTER_GATEWAY_URL",
        default_value = "http://localhost:4000"
    )]
    pub gateway_url: String,
    /// optional bootstrap config. Without a database it seeds the in-memory
    /// store; with `--database-url` its providers/routes become read-only
    /// "config models" merged over the DB-defined ones (config wins on
    /// name conflicts)
    #[arg(short, long, env = "ROLTER_CONFIG")]
    pub config: Option<PathBuf>,
    /// postgres connection string; when set, the control plane reads/serves
    /// its config from the database instead of the bootstrap toml
    #[cfg(feature = "postgres")]
    #[arg(long, env = "ROLTER_DATABASE_URL")]
    pub database_url: Option<String>,
    /// redis connection url; when set, config-version bumps are published on
    /// the `rolter.config` channel so gateways refetch immediately instead of
    /// waiting for their poll interval
    #[arg(long, env = "ROLTER_REDIS_URL")]
    pub redis_url: Option<String>,
    /// clickhouse http url; when set, the dashboard usage/cost analytics
    /// endpoints (`/api/v1/analytics/*`) query the `request_logs` table
    #[arg(long, env = "CLICKHOUSE_URL")]
    pub clickhouse_url: Option<String>,
    /// bearer token required on the CRUD API and `/internal/snapshot`; when
    /// unset those endpoints are open (a warning is logged at startup)
    #[arg(long, env = "ROLTER_ADMIN_TOKEN")]
    pub admin_token: Option<String>,
    /// bearer token required on `/internal/*` — the control↔data-plane channel,
    /// which carries decrypted provider credentials. When set, the operator
    /// admin token no longer opens that channel; when unset it falls back to
    /// `--admin-token` (the historical behavior)
    #[arg(long, env = "ROLTER_INTERNAL_TOKEN")]
    pub internal_token: Option<String>,
    /// when set (e.g. `127.0.0.1:4002`), `/internal/*` moves off the public API
    /// listener onto its own socket, so the plaintext-credential channel is not
    /// reachable on the port the dashboard and management API are served from
    #[arg(long, env = "ROLTER_INTERNAL_ADDR")]
    pub internal_addr: Option<SocketAddr>,
}

/// Names owned by the bootstrap config file: immutable at runtime,
/// LiteLLM-style. The CRUD API rejects mutations that collide with them.
// only read by the postgres-gated CRUD module
#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
#[derive(Default)]
struct ConfigOwned {
    providers: std::collections::HashSet<String>,
    models: std::collections::HashSet<String>,
    /// readonly provider-group slugs; the CRUD API rejects mutations against
    /// these (ADR-0022). default-tier groups are DB-owned and absent here
    groups: std::collections::HashSet<String>,
}

impl ConfigOwned {
    fn from_config(config: &GatewayConfig) -> Self {
        // config.providers / config.routes / config.provider_groups hold only the
        // readonly (effective) tier; the default tiers are seeded to the DB and
        // are deliberately editable, so they must not be tracked as config-owned
        Self {
            providers: config.providers.iter().map(|p| p.name.clone()).collect(),
            models: config.routes.iter().map(|r| r.model.clone()).collect(),
            groups: config
                .provider_groups
                .iter()
                .map(|g| {
                    g.slug
                        .clone()
                        .unwrap_or_else(|| rolter_core::slug::slugify(&g.name))
                })
                .collect(),
        }
    }
}

#[derive(Clone)]
struct ControlState {
    store: Arc<dyn ConfigStore>,
    /// provider/model names declared in the bootstrap config; read-only via
    /// the API (empty when no bootstrap config was given)
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    config_owned: Arc<ConfigOwned>,
    /// set when `--redis-url` is configured; config-version bumps are
    /// published on [`rolter_core::CONFIG_CHANNEL`] (best-effort)
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    redis: Option<redis::Client>,
    /// set when `--clickhouse-url` is configured; backs the usage/cost
    /// analytics endpoints
    clickhouse: Option<analytics::ClickHouseClient>,
    /// egress policy from the bootstrap config (defaults when there is none);
    /// the CRUD API rejects a provider whose `api_base` it denies, so an SSRF
    /// target never reaches the database in the first place
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    egress: Arc<rolter_core::EgressPolicy>,
    /// settlement currency + rate table from the bootstrap config (defaults
    /// when there is none); the CRUD API rejects a model price in a currency
    /// this cannot convert, so an unconvertible price never reaches the
    /// database and never silently charges the wrong amount (#650)
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    currency: Arc<rolter_core::CurrencyConfig>,
    /// when set, the CRUD API and `/internal/snapshot` require
    /// `Authorization: Bearer <token>`
    admin_token: Option<Arc<String>>,
    /// when set, `/internal/*` requires this token *instead of* the admin one,
    /// so an operator credential no longer unlocks decrypted provider keys.
    /// Falls back to `admin_token` when unset
    internal_token: Option<Arc<String>>,
    /// shared client for the `/gw/*` reverse proxy to the gateway data plane
    http: reqwest::Client,
    /// base URL of the rolter-gateway the `/gw/*` proxy forwards to
    gateway_url: Arc<String>,
    /// set when `--database-url` is configured; backs the CRUD API, which
    /// needs direct repository access beyond what `ConfigStore` exposes
    #[cfg(feature = "postgres")]
    pool: Option<sqlx::PgPool>,
    /// live cross-origin policy from `security_settings` (#813), swapped on
    /// every settings write so an edit applies without a restart. Empty by
    /// default, which is the same-origin deployment and adds no headers
    cors: Arc<arc_swap::ArcSwap<cors::CorsPolicy>>,
}

/// Run the control plane to completion. The caller owns argument parsing and
/// telemetry initialization.
pub async fn run(args: Args) -> anyhow::Result<()> {
    let bootstrap = match &args.config {
        Some(path) if path.exists() => Some(GatewayConfig::load(path)?),
        _ => None,
    };
    let config_owned = Arc::new(
        bootstrap
            .as_ref()
            .map(ConfigOwned::from_config)
            .unwrap_or_default(),
    );

    let redis = match &args.redis_url {
        Some(url) => match redis::Client::open(url.as_str()) {
            Ok(client) => {
                tracing::info!(%url, "publishing config bumps to redis");
                Some(client)
            }
            Err(err) => {
                tracing::warn!(error = %err, "invalid redis url; config pub/sub disabled");
                None
            }
        },
        None => None,
    };

    let clickhouse = args.clickhouse_url.as_deref().map(|url| {
        tracing::info!(%url, "usage/cost analytics enabled");
        analytics::ClickHouseClient::new(url)
    });

    let admin_token = args
        .admin_token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| Arc::new(t.to_string()));
    if admin_token.is_none() {
        tracing::warn!(
            "ROLTER_ADMIN_TOKEN is unset: the management API and /internal/snapshot are \
             unauthenticated; set it before exposing the control plane beyond localhost"
        );
    }

    let internal_token = args
        .internal_token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| Arc::new(t.to_string()));
    if internal_token.is_none() && admin_token.is_some() {
        tracing::warn!(
            "ROLTER_INTERNAL_TOKEN is unset: /internal/snapshot, which carries decrypted \
             provider credentials, accepts the operator admin token; set a distinct internal \
             token (and ROLTER_INTERNAL_ADDR) to separate the two trust boundaries"
        );
    }

    let egress = Arc::new(
        bootstrap
            .as_ref()
            .map(|c| c.egress.clone())
            .unwrap_or_default(),
    );
    let currency = Arc::new(
        bootstrap
            .as_ref()
            .map(|c| c.currency.clone())
            .unwrap_or_default(),
    );
    #[allow(unused_variables)]
    let (store, pool) = build_store(&args, bootstrap).await?;
    let http = reqwest::Client::new();
    let gateway_url = Arc::new(args.gateway_url.trim_end_matches('/').to_string());
    tracing::info!(gateway_url = %gateway_url, "proxying /gw/* to the gateway");
    #[cfg(feature = "postgres")]
    let state = ControlState {
        store,
        config_owned,
        redis,
        clickhouse,
        egress: egress.clone(),
        currency: currency.clone(),
        admin_token,
        internal_token,
        http,
        gateway_url,
        cors: Arc::default(),
        pool: pool.clone(),
    };
    #[cfg(not(feature = "postgres"))]
    let state = ControlState {
        store,
        config_owned,
        redis,
        clickhouse,
        egress: egress.clone(),
        currency: currency.clone(),
        admin_token,
        internal_token,
        http,
        gateway_url,
        cors: Arc::default(),
    };

    // converge the clickhouse ttl with the stored retention policy. spawned
    // rather than awaited: clickhouse is optional and may be slower to come up
    // than the control plane, and serving traffic must not wait on it
    #[cfg(feature = "postgres")]
    {
        let state = state.clone();
        tokio::spawn(async move { logging_settings::reconcile_retention(&state).await });
    }

    // load the cross-origin policy before the listener opens rather than in a
    // task: a split-origin dashboard hitting a control plane that has not read
    // its own settings yet would be refused, which looks exactly like the
    // misconfiguration this feature exists to fix (#813)
    #[cfg(feature = "postgres")]
    cors::refresh(&state).await;

    // when the operator gave /internal/* its own socket, it is not mounted on
    // the public router at all — the plaintext-credential channel is absent
    // from that surface rather than merely gated on it
    let split_internal = args.internal_addr.is_some();
    // the dashboard is built ahead of time, so per-deployment values are
    // injected into its html on the way out rather than baked in at build
    // time. rendered once: none of it can change without a restart
    // one kill switch covers every signal, the dashboard's browser tracing
    // included (#812): a deployment that turned telemetry off must not have the
    // SPA quietly shipping spans to a collector from the user's machine
    let telemetry_enabled = rolter_core::telemetry::export_enabled();
    if !telemetry_enabled {
        tracing::info!(
            "{} is off; no telemetry is exported",
            rolter_core::telemetry::TELEMETRY_ENABLED_ENV
        );
    }
    let ui_runtime = ui_config::UiRuntimeConfig {
        otel_endpoint: telemetry_enabled
            .then(|| args.ui_otel_endpoint.clone())
            .flatten(),
        otel_service_name: args.ui_otel_service_name.clone(),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };
    if ui_runtime.is_configured() {
        tracing::info!(
            endpoint = ui_runtime.otel_endpoint.as_deref().unwrap_or_default(),
            "dashboard browser tracing enabled"
        );
    }
    let index = load_index(&args.ui_dir, &ui_runtime);

    let app = build_app_with(state.clone(), !split_internal)
        // anything not matched by the api falls through to the built SPA.
        // ServeDir must not serve index.html on its own: every route that ends
        // at the SPA has to come through the fallback, or it arrives without
        // the injected runtime config and the dashboard's tracing stays inert.
        // Routing every miss there is also what lets a deep link like /models
        // survive a refresh instead of 404ing
        .fallback_service(
            ServeDir::new(&args.ui_dir)
                .append_index_html_on_directories(false)
                .fallback(get(move || {
                    let index = index.clone();
                    async move {
                        match index {
                            Some(html) => Html(html).into_response(),
                            None => StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                })),
        );

    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    tracing::info!(%addr, ui_dir = %args.ui_dir.display(), "rolter-control listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let Some(internal_addr) = args.internal_addr else {
        axum::serve(listener, app).await?;
        return Ok(());
    };

    let internal = internal_routes(&state).with_state(state);
    tracing::info!(addr = %internal_addr, "rolter-control internal api listening");
    let internal_listener = tokio::net::TcpListener::bind(internal_addr).await?;
    // both listeners share the process: if either dies the control plane is
    // degraded (no config propagation, or no dashboard), so exit rather than
    // limp on with half a control plane
    tokio::try_join!(async { axum::serve(listener, app).await }, async {
        axum::serve(internal_listener, internal).await
    },)?;
    Ok(())
}

/// Read the dashboard's `index.html` out of `ui_dir` and inject the runtime
/// config into it.
///
/// Returns `None` when the dashboard was never built into `ui_dir`. The control
/// plane is a perfectly useful API without a dashboard — `rolter-control` is
/// run headless in plenty of deployments — so a missing SPA is a warning and a
/// 404 on the dashboard routes, not a refusal to start.
fn load_index(ui_dir: &std::path::Path, config: &ui_config::UiRuntimeConfig) -> Option<String> {
    let path = ui_dir.join("index.html");
    match std::fs::read_to_string(&path) {
        Ok(html) => Some(ui_config::inject(&html, config)),
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                %err,
                "no dashboard index.html; serving the api without a dashboard"
            );
            None
        }
    }
}

/// Assemble the control-plane API router (no SPA fallback) with `state` applied.
/// The CRUD routes are only mounted when a postgres pool is present.
/// `mount_internal` controls whether `/internal/*` is served on this router at
/// all. It is `false` when the operator gave the internal routes their own
/// listener, so the plaintext-credential channel is simply absent from the
/// public API surface rather than merely token-gated.
fn build_app_with(state: ControlState, mount_internal: bool) -> Router {
    #[allow(unused_mut)]
    let mut api = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/api/v1/ping",
            get(|| async { Json(json!({"pong": true})) }),
        )
        .route("/api/v1/roles", get(list_roles))
        .route("/api/v1/config", get(get_config))
        .merge(analytics::router())
        .merge(health::router())
        // reverse-proxy the gateway data plane for the dashboard Playground;
        // authenticated by the virtual key the gateway itself checks
        .merge(proxy::router());
    // login is authenticated by the request body (email/password), not the
    // admin token, so /api/v1/auth/* sits on the open router alongside
    // everything else here; `me` still requires a valid session bearer token
    // via the `CurrentUser` extractor, it's just not gated by admin_token.
    //
    // the CRUD API enforces RBAC per handler (see `crate::rbac`): each handler
    // resolves a `Principal` and checks the caller's role at the resource's
    // scope, so it is NOT behind the blanket admin-token layer. open mode (no
    // admin token) is preserved inside the `Principal` extractor.
    #[cfg(feature = "postgres")]
    if state.pool.is_some() {
        alerting::start_evaluator(state.clone());
        // renew MCP OAuth sessions before they lapse, so a user consents once
        // rather than every hour (#707)
        mcp_oauth_flow::start_refresher(state.clone());
        api = api
            .merge(access_control::router())
            .merge(alerting::router())
            .merge(auth::router())
            .merge(auth_policy::router())
            .merge(invitations::router())
            .merge(crud::router())
            .merge(me::router())
            .merge(plugins::router())
            .merge(mcp_logs::router())
            .merge(ui_events::router())
            .merge(mcp_oauth::router())
            .merge(mcp_oauth_flow::router())
            .merge(feature_flags::router())
            .merge(guardrails::router())
            .merge(logging_settings::router())
            .merge(runtime_policy::router())
            .merge(compatibility_policy::router())
            .merge(client_settings::router())
            .merge(model_defaults::router())
            .merge(adaptive_policy::router())
            .merge(adaptive_telemetry::router())
            .merge(rbac_matrix::router())
            .merge(scim::router())
            .merge(scim_groups::router())
            .merge(sso::router())
            .merge(cluster::router())
            .merge(connectors::router())
            .merge(security::router());
    }

    if mount_internal {
        api = api.merge(internal_routes(&state));
    }
    // the cross-origin policy wraps everything, including the internal routes
    // and the SPA fallback, because a browser that is denied the API must be
    // denied it uniformly. it is inert until an operator configures an origin
    // (#813)
    api.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        cors::apply_cors,
    ))
    .with_state(state)
}

/// The control↔data-plane routes. They carry decrypted provider credentials,
/// so they sit behind the internal token (falling back to the admin token) and
/// can be served on their own listener instead of the public API port — see
/// [`Args::internal_addr`].
fn internal_routes(state: &ControlState) -> Router<ControlState> {
    #[allow(unused_mut)]
    let mut routes = Router::new().route("/internal/snapshot", get(get_snapshot));
    // the reverse direction of the same channel: gateways push adaptive-routing
    // samples the dashboard reads back out of the control plane (#751)
    #[cfg(feature = "postgres")]
    {
        routes = routes.merge(adaptive_telemetry::internal_router());
    }
    routes.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_internal_token,
    ))
}

/// Reject requests to `/internal/*` lacking the internal bearer token.
///
/// The expected token is `--internal-token` when set, so an operator holding
/// only the admin token cannot read decrypted provider credentials. It falls
/// back to the admin token when no internal token is configured (the historical
/// behavior, kept so an existing deployment does not break on upgrade), and
/// passes through when neither is set.
async fn require_internal_token(
    State(state): State<ControlState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let expected = state
        .internal_token
        .as_deref()
        .or(state.admin_token.as_deref());
    let Some(expected) = expected else {
        return next.run(request).await;
    };
    let presented = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    // constant-time comparison so the token can't be recovered byte by byte
    let matches: bool =
        subtle::ConstantTimeEq::ct_eq(presented.as_bytes(), expected.as_bytes()).into();
    if !matches {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"message": "missing or invalid internal token"}})),
        )
            .into_response();
    }
    next.run(request).await
}

/// Build a postgres-backed control-plane API router for integration tests.
/// Runs migrations on `pool`, mounts the full CRUD API and `/internal/snapshot`,
/// and omits redis/clickhouse and the SPA fallback. Intended to be served on an
/// ephemeral port by the test harness.
#[cfg(feature = "postgres")]
pub async fn test_app(pool: sqlx::PgPool) -> anyhow::Result<Router> {
    test_app_with_admin_token(pool, None).await
}

/// [`test_app`] with an admin token, for exercising the auth guard on the
/// CRUD API and `/internal/snapshot`.
#[cfg(feature = "postgres")]
pub async fn test_app_with_admin_token(
    pool: sqlx::PgPool,
    admin_token: Option<String>,
) -> anyhow::Result<Router> {
    rolter_store::postgres::run_migrations(&pool).await?;
    let store: Arc<dyn ConfigStore> =
        Arc::new(rolter_store::PostgresConfigStore::new(pool.clone()));
    let state = ControlState {
        store,
        config_owned: Arc::new(ConfigOwned::default()),
        redis: None,
        clickhouse: None,
        egress: Arc::new(Default::default()),
        currency: Arc::new(Default::default()),
        admin_token: admin_token.map(Arc::new),
        internal_token: None,
        http: reqwest::Client::new(),
        gateway_url: Arc::new("http://localhost:4000".to_string()),
        cors: Arc::default(),
        pool: Some(pool),
    };
    Ok(build_app_with(state, true))
}

/// Build the config store: postgres-backed when `--database-url` is set
/// (running migrations first; a bootstrap config, when given, is layered on
/// top as read-only config models via [`MergedConfigStore`]), otherwise an
/// in-memory store seeded from the bootstrap toml. Also returns the raw pool
/// (postgres builds only), which the CRUD API needs for direct repository
/// access.
#[cfg(feature = "postgres")]
async fn build_store(
    args: &Args,
    bootstrap: Option<GatewayConfig>,
) -> anyhow::Result<(Arc<dyn ConfigStore>, Option<sqlx::PgPool>)> {
    if let Some(database_url) = &args.database_url {
        let pool = rolter_store::postgres::connect(database_url).await?;
        rolter_store::postgres::run_migrations(&pool).await?;
        if let Some(config) = bootstrap.as_ref() {
            // providers first: a models.default target or a provider_groups.default
            // member may reference a provider that providers.default just seeded
            seed_default_providers(&pool, config).await?;
            seed_default_provider_groups(&pool, config).await?;
            seed_default_models(&pool, config).await?;
        }
        let db_store: Arc<dyn ConfigStore> =
            Arc::new(rolter_store::PostgresConfigStore::new(pool.clone()));
        let store: Arc<dyn ConfigStore> = match bootstrap {
            Some(config) => Arc::new(MergedConfigStore::new(config, db_store)),
            None => db_store,
        };
        return Ok((store, Some(pool)));
    }

    let config = bootstrap.unwrap_or_default();
    Ok((Arc::new(InMemoryConfigStore::new(config)), None))
}

/// Seed editable `[[models.default]]` routes exactly once. Defaults deliberately
/// target the bootstrap `default/default/default` tenancy created by `rolter
/// seed`; a deployment without that project is left untouched rather than
/// guessing a tenant. Existing rows are never overwritten on a restart.
#[cfg(feature = "postgres")]
async fn seed_default_models(pool: &sqlx::PgPool, config: &GatewayConfig) -> anyhow::Result<()> {
    if config.models.defaults.is_empty() {
        return Ok(());
    }
    let project: Option<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        "select p.id, o.id from projects p \
         join teams t on t.id = p.team_id \
         join orgs o on o.id = t.org_id \
         where o.slug = 'default' and t.name = 'default' and p.name = 'default' \
         limit 1",
    )
    .fetch_optional(pool)
    .await?;
    let Some((project_id, org_id)) = project else {
        tracing::warn!(
            "models.default was not seeded: create the default org/team/project first with rolter-seed"
        );
        return Ok(());
    };
    let routes = rolter_store::postgres::repo::RouteRepo(pool);
    let targets = rolter_store::postgres::repo::RouteTargetRepo(pool);
    let providers = rolter_store::postgres::repo::ProviderRepo(pool)
        .list(org_id)
        .await?;
    for route in &config.models.defaults {
        if routes
            .list(project_id)
            .await?
            .iter()
            .any(|existing| existing.model == route.model)
        {
            continue;
        }
        let strategy = match route.strategy {
            rolter_core::BalancingStrategy::RoundRobin => "round_robin",
            rolter_core::BalancingStrategy::Random => "random",
            rolter_core::BalancingStrategy::PowerOfTwo => "power_of_two",
            rolter_core::BalancingStrategy::ConsistentHash => "consistent_hash",
            rolter_core::BalancingStrategy::CacheAware => "cache_aware",
            rolter_core::BalancingStrategy::Weighted => "weighted",
            rolter_core::BalancingStrategy::Pipeline => "pipeline",
            rolter_core::BalancingStrategy::Cheapest => "cheapest",
            rolter_core::BalancingStrategy::Fastest => "fastest",
            rolter_core::BalancingStrategy::PreciseCacheAware => "precise_cache_aware",
            rolter_core::BalancingStrategy::LmcacheAware => "lmcache_aware",
            rolter_core::BalancingStrategy::Adaptive => "adaptive",
        };
        let created = routes.create(project_id, &route.model, strategy).await?;
        let params = serde_json::to_value(&route.params)?;
        let policy = serde_json::to_value(&route.param_policy)?;
        routes.set_params(created.id, &params, &policy).await?;
        for target in &route.targets {
            if let Some(provider) = providers.iter().find(|p| p.name == target.provider) {
                targets
                    .create(
                        created.id,
                        provider.id,
                        target.model.as_deref(),
                        target.weight as i32,
                    )
                    .await?;
            } else {
                tracing::warn!(
                    model = %route.model,
                    provider = %target.provider,
                    "models.default target was not seeded because the provider is not DB-owned"
                );
            }
        }
        tracing::info!(model = %route.model, "seeded editable default model");
    }
    Ok(())
}

/// Resolve the bootstrap `default/default/default` `(project_id, org_id)`, or
/// `None` when it has not been created yet (a deployment without `rolter-seed`
/// is left untouched rather than guessing a tenant).
#[cfg(feature = "postgres")]
async fn default_project(pool: &sqlx::PgPool) -> anyhow::Result<Option<(uuid::Uuid, uuid::Uuid)>> {
    Ok(sqlx::query_as(
        "select p.id, o.id from projects p \
         join teams t on t.id = p.team_id \
         join orgs o on o.id = t.org_id \
         where o.slug = 'default' and t.name = 'default' and p.name = 'default' \
         limit 1",
    )
    .fetch_optional(pool)
    .await?)
}

/// The database `kind` string for a [`rolter_core::ProviderKind`], matching the
/// values decoded in `rolter_store::postgres`.
#[cfg(feature = "postgres")]
fn provider_kind_str(kind: &rolter_core::ProviderKind) -> &'static str {
    use rolter_core::ProviderKind::*;
    match kind {
        Openai => "openai",
        Anthropic => "anthropic",
        OpenaiCompatible => "openai_compatible",
        Ollama => "ollama",
        OllamaCloud => "ollama_cloud",
        LlamaCpp => "llama_cpp",
        Openrouter => "openrouter",
        Tei => "tei",
        AzureOpenai => "azure_openai",
        Bedrock => "bedrock",
        Vertex => "vertex",
        Gemini => "gemini",
        GeminiNative => "gemini_native",
        GeminiInteractions => "gemini_interactions",
        Mistral => "mistral",
        Groq => "groq",
        Xai => "xai",
        MetaLlamaApi => "meta_llama_api",
        Cohere => "cohere",
        Perplexity => "perplexity",
        Together => "together",
        Fireworks => "fireworks",
        Databricks => "databricks",
        AlephAlpha => "aleph_alpha",
        Nebius => "nebius",
        Ovhcloud => "ovhcloud",
        Scaleway => "scaleway",
        Deepseek => "deepseek",
        Qwen => "qwen",
        Zhipu => "zhipu",
        Kimi => "kimi",
        Ernie => "ernie",
        Doubao => "doubao",
        Hunyuan => "hunyuan",
        Yi => "yi",
        Minimax => "minimax",
        Baichuan => "baichuan",
        Gigachat => "gigachat",
        YandexGpt => "yandex_gpt",
        CloudRu => "cloud_ru",
        MtsAi => "mts_ai",
        Naver => "naver",
        Upstage => "upstage",
        Rinna => "rinna",
        Rakuten => "rakuten",
        Sarvam => "sarvam",
        Krutrim => "krutrim",
        Falcon => "falcon",
    }
}

/// Seed editable `[providers.default]` providers exactly once (ADR-0022). Like
/// `seed_default_models`, defaults target the bootstrap `default/default/default`
/// org; an existing provider with the same slug or name is never overwritten.
/// Seeded providers are DB-owned and editable — not config-owned.
#[cfg(feature = "postgres")]
async fn seed_default_providers(pool: &sqlx::PgPool, config: &GatewayConfig) -> anyhow::Result<()> {
    if config.provider_defaults.is_empty() {
        return Ok(());
    }
    let Some((_project_id, org_id)) = default_project(pool).await? else {
        tracing::warn!(
            "providers.default was not seeded: create the default org/team/project first with rolter-seed"
        );
        return Ok(());
    };
    let repo = rolter_store::postgres::repo::ProviderRepo(pool);
    let keys = rolter_store::postgres::repo::ProviderKeyRepo(pool);
    let existing = repo.list(org_id).await?;
    for provider in &config.provider_defaults {
        let slug = provider
            .slug
            .clone()
            .unwrap_or_else(|| rolter_core::slug::slugify(&provider.name));
        if existing
            .iter()
            .any(|p| p.name == provider.name || p.slug == slug)
        {
            continue;
        }
        let row = repo
            .create(
                org_id,
                &provider.name,
                &slug,
                provider_kind_str(&provider.kind),
                &provider.api_base,
                provider.api_key_env.as_deref(),
                provider.egress_proxy.as_deref(),
                &provider.egress_proxies,
            )
            .await?;
        // seal an inline api_key at rest; api_key_env stays a plaintext var name
        if let Some(api_key) = provider.api_key.as_deref() {
            use rolter_store::postgres::crypto::{Kek, KEK_ENV};
            match Kek::from_env() {
                Some(kek) => {
                    let (ciphertext, nonce) = kek.encrypt(api_key)?;
                    keys.set(row.id, &ciphertext, &nonce).await?;
                }
                None => tracing::warn!(
                    provider = %provider.name,
                    "providers.default inline api_key not sealed: {KEK_ENV} is unset; set api_key_env or the KEK"
                ),
            }
        }
        tracing::info!(provider = %provider.name, %slug, "seeded editable default provider");
    }
    Ok(())
}

/// Seed editable `[provider_groups.default]` groups exactly once (ADR-0022).
/// Like the other default seeds, groups target the bootstrap default org and an
/// existing group with the same slug is never overwritten. A member referencing
/// a provider that is not DB-owned is skipped with a warning. Runs after the
/// provider seed so a group member can reference a just-seeded provider.
#[cfg(feature = "postgres")]
async fn seed_default_provider_groups(
    pool: &sqlx::PgPool,
    config: &GatewayConfig,
) -> anyhow::Result<()> {
    if config.provider_group_defaults.is_empty() {
        return Ok(());
    }
    let Some((_project_id, org_id)) = default_project(pool).await? else {
        tracing::warn!(
            "provider_groups.default was not seeded: create the default org/team/project first with rolter-seed"
        );
        return Ok(());
    };
    let groups = rolter_store::postgres::repo::ProviderGroupRepo(pool);
    let providers = rolter_store::postgres::repo::ProviderRepo(pool)
        .list(org_id)
        .await?;
    let existing = groups.list(org_id).await?;
    for group in &config.provider_group_defaults {
        let slug = group
            .slug
            .clone()
            .unwrap_or_else(|| rolter_core::slug::slugify(&group.name));
        if existing.iter().any(|g| g.slug == slug) {
            continue;
        }
        let strategy = balancing_strategy_str(group.strategy);
        let created = groups.create(org_id, &group.name, &slug, strategy).await?;
        let mut members = Vec::with_capacity(group.members.len());
        for member in &group.members {
            match providers.iter().find(|p| p.name == member.provider) {
                Some(provider) => members.push((
                    provider.id,
                    member.model.clone(),
                    member.weight.max(1) as i32,
                )),
                None => tracing::warn!(
                    group = %group.name,
                    provider = %member.provider,
                    "provider_groups.default member skipped: provider is not DB-owned"
                ),
            }
        }
        groups.set_members(created.id, &members).await?;
        tracing::info!(group = %group.name, %slug, "seeded editable default provider group");
    }
    Ok(())
}

/// The database `strategy` string for a [`rolter_core::BalancingStrategy`],
/// matching the values `rolter_store::postgres::parse_strategy` decodes.
#[cfg(feature = "postgres")]
fn balancing_strategy_str(strategy: rolter_core::BalancingStrategy) -> &'static str {
    use rolter_core::BalancingStrategy::*;
    match strategy {
        RoundRobin => "round_robin",
        Random => "random",
        PowerOfTwo => "power_of_two",
        ConsistentHash => "consistent_hash",
        CacheAware => "cache_aware",
        Weighted => "weighted",
        Pipeline => "pipeline",
        Cheapest => "cheapest",
        Fastest => "fastest",
        PreciseCacheAware => "precise_cache_aware",
        LmcacheAware => "lmcache_aware",
        Adaptive => "adaptive",
    }
}

#[cfg(not(feature = "postgres"))]
async fn build_store(
    _args: &Args,
    bootstrap: Option<GatewayConfig>,
) -> anyhow::Result<(Arc<dyn ConfigStore>, Option<()>)> {
    let config = bootstrap.unwrap_or_default();
    Ok((Arc::new(InMemoryConfigStore::new(config)), None))
}

async fn list_roles() -> Json<Value> {
    let roles = [Role::Admin, Role::Member, Role::Viewer];
    Json(serde_json::to_value(roles).unwrap_or_default())
}

async fn get_config(State(state): State<ControlState>) -> Json<GatewayConfig> {
    let mut config = state.store.load().await.unwrap_or_default();
    // this endpoint feeds the dashboard; upstream credentials stay between the
    // store and the gateway (via the token-guarded snapshot endpoint)
    for provider in &mut config.providers {
        provider.api_key = None;
        for key in &mut provider.api_keys {
            key.key = None;
        }
    }
    Json(config)
}

#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    /// the gateway's last-seen config version; if it's already current, the
    /// control plane replies `304 Not Modified` with no body
    version: Option<i64>,
}

/// Attach the polling node's operator-requested state, so a drain reaches it on
/// the channel it already polls. Absent when the caller is not a known node.
#[cfg(feature = "postgres")]
fn with_node_state(mut response: Response, desired_state: &Option<String>) -> Response {
    if let Some(value) = desired_state
        .as_deref()
        .and_then(|state| axum::http::HeaderValue::from_str(state).ok())
    {
        response
            .headers_mut()
            .insert(cluster::NODE_STATE_HEADER, value);
    }
    response
}

/// Without the store there is no inventory, so nothing is attached.
#[cfg(not(feature = "postgres"))]
fn with_node_state(response: Response, _desired_state: &Option<String>) -> Response {
    response
}

/// Runtime snapshot endpoint gateways poll to pick up config changes without
/// a restart. Returns `{"version": N, "config": GatewayConfig}`, or `304` if
/// the caller's `version` is already current.
async fn get_snapshot(
    State(state): State<ControlState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<SnapshotQuery>,
) -> Response {
    // snapshot generation is fleet-wide propagation delay: every gateway waits
    // on it for its config, and until #809 it was invisible. the stage span
    // costs nothing when no OTLP pipeline is installed
    let _span = rolter_core::stage_span!("snapshot.build");
    // a node that identifies itself is recorded in the cluster inventory; the
    // poll it already makes is the heartbeat, so there is no second channel
    #[cfg(feature = "postgres")]
    let desired_state = cluster::record_heartbeat(&state, &headers, query.version).await;
    #[cfg(not(feature = "postgres"))]
    let desired_state: Option<String> = None;
    let _ = &headers;
    let version = match state.store.current_version().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"message": err.to_string()}})),
            )
                .into_response()
        }
    };
    if query.version.is_some_and(|requested| requested >= version) {
        return with_node_state(StatusCode::NOT_MODIFIED.into_response(), &desired_state);
    }
    match state.store.load().await {
        Ok(mut config) => {
            // prune rows that are only unservable by themselves (a route whose
            // targets aren't set up yet) so one half-built entry can't 500 the
            // snapshot and freeze config propagation for every other tenant
            let _load = rolter_core::stage_span!("snapshot.sanitize");
            let omitted = config.sanitize_for_snapshot();
            if !omitted.is_empty() {
                tracing::warn!(
                    ?omitted,
                    "omitted unservable entries from the config snapshot"
                );
            }
            // never distribute a broken config to gateways: reply with the
            // problems instead so the operator can fix the source
            if let Err(problems) = config.validate() {
                tracing::error!(?problems, "refusing to serve invalid config snapshot");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {
                        "message": "config failed validation",
                        "problems": problems,
                    }})),
                )
                    .into_response();
            }
            with_node_state(
                Json(json!({"version": version, "config": config})).into_response(),
                &desired_state,
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"message": err.to_string()}})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch `ui_dir`, removed when the guard drops. No `tempfile` in this
    /// crate's dev-dependencies, and one directory is not worth adding one.
    struct ScratchUiDir(std::path::PathBuf);

    impl ScratchUiDir {
        fn with_index(name: &str, html: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rolter-ui-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create scratch ui dir");
            std::fs::write(dir.join("index.html"), html).expect("write index.html");
            Self(dir)
        }
    }

    impl Drop for ScratchUiDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_index_injects_the_runtime_config_into_the_built_dashboard() {
        let dir = ScratchUiDir::with_index(
            "injects",
            "<!doctype html><html><head><title>rolter</title></head><body></body></html>",
        );
        let config = ui_config::UiRuntimeConfig {
            otel_endpoint: Some("http://localhost:4318/v1/traces".to_string()),
            ..Default::default()
        };

        let html = load_index(&dir.0, &config).expect("index.html was read");

        assert!(html.contains("window.__ROLTER_CONFIG__"), "{html}");
        assert!(html.contains("http://localhost:4318/v1/traces"), "{html}");
        assert!(
            html.contains("<title>rolter</title>"),
            "original kept: {html}"
        );
    }

    #[test]
    fn load_index_is_none_when_the_dashboard_was_never_built() {
        let missing = std::env::temp_dir().join(format!("rolter-no-ui-{}", std::process::id()));
        // a headless control plane is a supported deployment, so this must be
        // an absent dashboard rather than a startup failure
        assert!(load_index(&missing, &ui_config::UiRuntimeConfig::default()).is_none());
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn provider_kind_str_covers_every_kind_and_matches_the_decoder() {
        use rolter_core::ProviderKind::*;
        // each variant maps to the exact string rolter_store decodes back
        for kind in [
            Openai,
            Anthropic,
            OpenaiCompatible,
            Ollama,
            OllamaCloud,
            LlamaCpp,
            Openrouter,
            Tei,
            AzureOpenai,
            Bedrock,
            Vertex,
            Gemini,
            GeminiNative,
            Mistral,
            Groq,
            Xai,
            MetaLlamaApi,
            Cohere,
            Perplexity,
            Together,
            Fireworks,
            Databricks,
            AlephAlpha,
            Nebius,
            Ovhcloud,
            Scaleway,
            Deepseek,
            Qwen,
            Zhipu,
            Kimi,
            Ernie,
            Doubao,
            Hunyuan,
            Yi,
            Minimax,
            Baichuan,
            Gigachat,
            YandexGpt,
            CloudRu,
            MtsAi,
            Naver,
            Upstage,
            Rinna,
            Rakuten,
            Sarvam,
            Krutrim,
            Falcon,
        ] {
            let s = provider_kind_str(&kind);
            assert!(!s.is_empty());
        }
        assert_eq!(provider_kind_str(&OpenaiCompatible), "openai_compatible");
        assert_eq!(provider_kind_str(&AzureOpenai), "azure_openai");
    }

    fn state_with_token(token: Option<&str>) -> ControlState {
        state_with_tokens(token, None)
    }

    fn state_with_tokens(admin: Option<&str>, internal: Option<&str>) -> ControlState {
        ControlState {
            store: Arc::new(InMemoryConfigStore::new(GatewayConfig::default())),
            config_owned: Arc::new(ConfigOwned::default()),
            egress: Arc::new(Default::default()),
            currency: Arc::new(Default::default()),
            redis: None,
            clickhouse: None,
            admin_token: admin.map(|t| Arc::new(t.to_string())),
            internal_token: internal.map(|t| Arc::new(t.to_string())),
            http: reqwest::Client::new(),
            gateway_url: Arc::new("http://localhost:4000".to_string()),
            cors: Arc::default(),
            #[cfg(feature = "postgres")]
            pool: None,
        }
    }

    /// the default deployment shape: /internal/* on the same listener as the
    /// public api, gated by a token
    fn build_app_with_internal(state: ControlState) -> Router {
        build_app_with(state, true)
    }

    async fn serve(app: Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn snapshot_requires_admin_token_when_configured() {
        let addr = serve(build_app_with_internal(state_with_token(Some("sekrit")))).await;
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/internal/snapshot");

        let unauthenticated = client.get(&url).send().await.unwrap();
        assert_eq!(unauthenticated.status(), 401);

        let wrong = client.get(&url).bearer_auth("nope").send().await.unwrap();
        assert_eq!(wrong.status(), 401);

        let ok = client.get(&url).bearer_auth("sekrit").send().await.unwrap();
        assert_eq!(ok.status(), 200);

        // the rest of the api stays open (dashboard reads, health)
        let ping = client
            .get(format!("http://{addr}/api/v1/ping"))
            .send()
            .await
            .unwrap();
        assert_eq!(ping.status(), 200);
    }

    #[tokio::test]
    async fn a_distinct_internal_token_locks_the_admin_token_out_of_the_snapshot() {
        // the snapshot carries decrypted provider credentials (#636): once the
        // operator separates the two trust boundaries, an admin credential must
        // not open that channel any more
        let addr = serve(build_app_with_internal(state_with_tokens(
            Some("operator"),
            Some("machine"),
        )))
        .await;
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/internal/snapshot");

        let admin = client
            .get(&url)
            .bearer_auth("operator")
            .send()
            .await
            .unwrap();
        assert_eq!(
            admin.status(),
            401,
            "admin token still unlocked the snapshot"
        );

        let machine = client
            .get(&url)
            .bearer_auth("machine")
            .send()
            .await
            .unwrap();
        assert_eq!(machine.status(), 200);
    }

    #[tokio::test]
    async fn a_separate_internal_listener_removes_the_snapshot_from_the_public_api() {
        // not merely gated on the public surface — absent from it
        let state = state_with_tokens(Some("operator"), Some("machine"));
        let public = serve(build_app_with(state.clone(), false)).await;
        let internal = serve(internal_routes(&state).with_state(state)).await;
        let client = reqwest::Client::new();

        let on_public = client
            .get(format!("http://{public}/internal/snapshot"))
            .bearer_auth("machine")
            .send()
            .await
            .unwrap();
        assert_eq!(on_public.status(), 404);

        // the public api is otherwise intact
        let ping = client
            .get(format!("http://{public}/api/v1/ping"))
            .send()
            .await
            .unwrap();
        assert_eq!(ping.status(), 200);

        let on_internal = client
            .get(format!("http://{internal}/internal/snapshot"))
            .bearer_auth("machine")
            .send()
            .await
            .unwrap();
        assert_eq!(on_internal.status(), 200);
    }

    #[tokio::test]
    async fn snapshot_open_when_no_token_configured() {
        let addr = serve(build_app_with_internal(state_with_token(None))).await;
        let resp = reqwest::Client::new()
            .get(format!("http://{addr}/internal/snapshot"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn gw_proxies_http_to_the_gateway() {
        // stand-in for the gateway data plane
        let upstream = Router::new()
            .route("/v1/ping", get(|| async { "pong-from-gw" }))
            .route(
                "/v1/echo",
                axum::routing::post(|body: String| async move { body }),
            );
        let up_addr = serve(upstream).await;

        let state = ControlState {
            gateway_url: Arc::new(format!("http://{up_addr}")),
            cors: Arc::default(),
            ..state_with_token(None)
        };
        let addr = serve(build_app_with(state, true)).await;
        let client = reqwest::Client::new();

        // GET forwards path + response body
        let got = client
            .get(format!("http://{addr}/gw/v1/ping"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(got, "pong-from-gw");

        // POST forwards the request body
        let echoed = client
            .post(format!("http://{addr}/gw/v1/echo"))
            .body("hello gateway")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(echoed, "hello gateway");
    }

    #[tokio::test]
    async fn config_endpoint_redacts_provider_keys() {
        let mut config = GatewayConfig::default();
        config.providers.push(rolter_core::ProviderConfig {
            name: "openai".to_string(),
            slug: None,
            kind: rolter_core::ProviderKind::Openai,
            api_base: "https://api.openai.com".to_string(),
            api_key: Some("sk-super-secret".to_string()),
            api_key_env: None,
            egress_proxy: None,
            egress_proxies: Vec::new(),
            kv_events: None,
            lmcache: None,
            ca_bundles: None,
            api_keys: vec![rolter_core::ApiKeyConfig {
                key: Some("sk-also-secret".to_string()),
                env: None,
                weight: 1,
            }],
            also_track_via_llm_call: false,
            llm_probe_model: None,
            status_page_url: None,
            role_profile: None,
            model_role_profiles: Default::default(),
        });
        let state = ControlState {
            store: Arc::new(InMemoryConfigStore::new(config)),
            ..state_with_token(None)
        };
        let addr = serve(build_app_with(state, true)).await;

        let body = reqwest::Client::new()
            .get(format!("http://{addr}/api/v1/config"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            !body.contains("sk-super-secret") && !body.contains("sk-also-secret"),
            "config endpoint must not leak provider keys: {body}"
        );
    }

    /// The #813 regression: an operator saves an origin and it must actually
    /// take effect, end to end through the real router.
    #[tokio::test]
    async fn a_configured_origin_gets_cors_headers_and_others_do_not() {
        let state = state_with_token(None);
        state.cors.store(Arc::new(cors::CorsPolicy::new(
            &["https://dash.example.com".to_string()],
            &["x-tenant".to_string()],
        )));
        let addr = serve(build_app_with(state, false)).await;
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/api/v1/ping");

        let allowed = client
            .get(&url)
            .header("origin", "https://dash.example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(
            allowed
                .headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://dash.example.com")
        );
        // Vary must be set so a shared cache cannot serve one origin's answer
        // to another
        assert!(allowed
            .headers()
            .get("vary")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("origin")));

        let denied = client
            .get(&url)
            .header("origin", "https://evil.example.com")
            .send()
            .await
            .unwrap();
        assert!(denied
            .headers()
            .get("access-control-allow-origin")
            .is_none());
        // still served, just not readable cross-origin — the browser enforces it
        assert_eq!(denied.status(), 200);
    }

    #[tokio::test]
    async fn a_preflight_is_answered_with_the_configured_and_trace_headers() {
        let state = state_with_token(None);
        state.cors.store(Arc::new(cors::CorsPolicy::new(
            &["https://dash.example.com".to_string()],
            &["x-tenant".to_string()],
        )));
        let addr = serve(build_app_with(state, false)).await;

        let response = reqwest::Client::new()
            .request(
                reqwest::Method::OPTIONS,
                format!("http://{addr}/api/v1/ping"),
            )
            .header("origin", "https://dash.example.com")
            .header("access-control-request-method", "GET")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 204);
        let allow = response
            .headers()
            .get("access-control-allow-headers")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        // #805: without these the dashboard's spans cannot join the gateway's
        assert!(allow.contains("traceparent"), "got: {allow}");
        assert!(allow.contains("tracestate"), "got: {allow}");
        assert!(allow.contains("x-tenant"), "got: {allow}");
    }

    #[tokio::test]
    async fn the_default_deployment_is_untouched_by_cors() {
        // no origins configured is the same-origin deployment everyone runs; it
        // must behave exactly as it did before the middleware existed
        let addr = serve(build_app_with(state_with_token(None), false)).await;
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/api/v1/ping"))
            .header("origin", "https://dash.example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
        assert!(response.headers().get("vary").is_none());
    }
}
