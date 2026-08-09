//! idempotent database bootstrap shared by the `rolter-seed` binary and the
//! unified launcher's `easy-up` subcommand.
//!
//! [`seed`] creates an org (+ a `default`/`default` team/project), an optional
//! admin user, and imports providers/routes from a bootstrap `rolter.toml`.
//! Every step checks for an existing row first, so re-running is a no-op.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use rolter_core::{BalancingStrategy, GatewayConfig, PromptTemplate, ProviderKind};
use rolter_store::postgres::repo::{
    OrgRepo, ProjectRepo, ProviderRepo, RouteRepo, RouteTargetRepo, TeamRepo,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Options controlling what [`seed`] creates. `org` defaults to `default` when
/// left empty; `org_slug` defaults to a slugified `org`.
#[derive(Debug, Clone, Default)]
pub struct SeedOptions {
    pub org: String,
    pub org_slug: Option<String>,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub import: Option<PathBuf>,
}

/// What [`seed`] found or created — enough for a caller to print a summary
/// without re-querying (never carries the admin password).
#[derive(Debug, Clone)]
pub struct SeedSummary {
    pub org_name: String,
    pub org_slug: String,
    pub admin_email: Option<String>,
    pub admin_created: bool,
}

/// Idempotently bootstrap `pool`: org, default team/project, optional admin,
/// optional bootstrap-toml import. Assumes migrations have already run.
pub async fn seed(pool: &PgPool, opts: &SeedOptions) -> anyhow::Result<SeedSummary> {
    let org_name = if opts.org.trim().is_empty() {
        "default"
    } else {
        opts.org.trim()
    };
    let org_slug = opts.org_slug.clone().unwrap_or_else(|| slugify(org_name));

    let orgs = OrgRepo(pool);
    let org = match orgs.list().await?.into_iter().find(|o| o.slug == org_slug) {
        Some(existing) => {
            tracing::info!(org = %existing.name, "org already exists, reusing");
            existing
        }
        None => {
            let created = orgs.create(org_name, &org_slug).await?;
            tracing::info!(org = %created.name, "created org");
            created
        }
    };

    let teams = TeamRepo(pool);
    let team = match teams
        .list(org.id)
        .await?
        .into_iter()
        .find(|t| t.name == "default")
    {
        Some(t) => t,
        None => teams.create(org.id, "default").await?,
    };

    let projects = ProjectRepo(pool);
    let project = match projects
        .list(team.id)
        .await?
        .into_iter()
        .find(|p| p.name == "default")
    {
        Some(p) => p,
        None => projects.create(team.id, "default").await?,
    };

    let mut admin_created = false;
    if let (Some(email), Some(password)) = (&opts.admin_email, &opts.admin_password) {
        admin_created = create_admin(pool, email, password).await?;
    }

    if let Some(path) = &opts.import {
        import_bootstrap_toml(pool, org.id, project.id, path).await?;
    }

    Ok(SeedSummary {
        org_name: org.name,
        org_slug: org.slug,
        admin_email: opts.admin_email.clone(),
        admin_created,
    })
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .fold(String::new(), |mut acc, s| {
            if !acc.is_empty() {
                acc.push('-');
            }
            acc.push_str(s);
            acc
        })
}

/// Create a superadmin user; returns `true` when a new row was inserted and
/// `false` when one already existed for `email`.
async fn create_admin(pool: &PgPool, email: &str, password: &str) -> anyhow::Result<bool> {
    let existing: Option<Uuid> = sqlx::query_scalar("select id from users where email = $1")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    if existing.is_some() {
        tracing::info!(%email, "admin user already exists, skipping");
        return Ok(false);
    }

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash admin password: {e}"))?
        .to_string();

    sqlx::query("insert into users (email, password_hash, is_superadmin) values ($1, $2, true)")
        .bind(email)
        .bind(hash)
        .execute(pool)
        .await?;
    tracing::info!(%email, "created admin user");
    Ok(true)
}

async fn import_bootstrap_toml(
    pool: &PgPool,
    org_id: Uuid,
    project_id: Uuid,
    path: &Path,
) -> anyhow::Result<()> {
    let config = GatewayConfig::load(path)?;
    let providers = ProviderRepo(pool);
    let routes = RouteRepo(pool);
    let targets = RouteTargetRepo(pool);

    let mut provider_ids = HashMap::new();
    for p in &config.providers {
        let kind = match p.kind {
            ProviderKind::Openai => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenaiCompatible => "openai_compatible",
            ProviderKind::Ollama => "ollama",
            ProviderKind::OllamaCloud => "ollama_cloud",
            ProviderKind::LlamaCpp => "llama_cpp",
            ProviderKind::Openrouter => "openrouter",
            ProviderKind::Tei => "tei",
            ProviderKind::AzureOpenai => "azure_openai",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::Vertex => "vertex",
            ProviderKind::Gemini => "gemini",
            ProviderKind::GeminiNative => "gemini_native",
            ProviderKind::GeminiInteractions => "gemini_interactions",
            ProviderKind::Mistral => "mistral",
            ProviderKind::Groq => "groq",
            ProviderKind::Xai => "xai",
            ProviderKind::MetaLlamaApi => "meta_llama_api",
            ProviderKind::Cohere => "cohere",
            ProviderKind::Perplexity => "perplexity",
            ProviderKind::Together => "together",
            ProviderKind::Fireworks => "fireworks",
            ProviderKind::Databricks => "databricks",
            ProviderKind::AlephAlpha => "aleph_alpha",
            ProviderKind::Nebius => "nebius",
            ProviderKind::Ovhcloud => "ovhcloud",
            ProviderKind::Scaleway => "scaleway",
            ProviderKind::Deepseek => "deepseek",
            ProviderKind::Qwen => "qwen",
            ProviderKind::Zhipu => "zhipu",
            ProviderKind::Kimi => "kimi",
            ProviderKind::Ernie => "ernie",
            ProviderKind::Doubao => "doubao",
            ProviderKind::Hunyuan => "hunyuan",
            ProviderKind::Yi => "yi",
            ProviderKind::Minimax => "minimax",
            ProviderKind::Baichuan => "baichuan",
            ProviderKind::Gigachat => "gigachat",
            ProviderKind::YandexGpt => "yandex_gpt",
            ProviderKind::CloudRu => "cloud_ru",
            ProviderKind::MtsAi => "mts_ai",
            ProviderKind::Naver => "naver",
            ProviderKind::Upstage => "upstage",
            ProviderKind::Rinna => "rinna",
            ProviderKind::Rakuten => "rakuten",
            ProviderKind::Sarvam => "sarvam",
            ProviderKind::Krutrim => "krutrim",
            ProviderKind::Falcon => "falcon",
        };
        let existing = providers
            .list(org_id)
            .await?
            .into_iter()
            .find(|row| row.name == p.name);
        let row = match existing {
            Some(row) => row,
            None => {
                let slug = p.slug.clone().unwrap_or_else(|| slugify(&p.name));
                providers
                    .create(
                        org_id,
                        &p.name,
                        &slug,
                        kind,
                        &p.api_base,
                        p.api_key_env.as_deref(),
                        p.egress_proxy.as_deref(),
                        &p.egress_proxies,
                    )
                    .await?
            }
        };
        provider_ids.insert(p.name.clone(), row.id);
        tracing::info!(provider = %p.name, "imported provider");
    }

    for r in &config.routes {
        let strategy = match r.strategy {
            BalancingStrategy::RoundRobin => "round_robin",
            BalancingStrategy::Random => "random",
            BalancingStrategy::PowerOfTwo => "power_of_two",
            BalancingStrategy::ConsistentHash => "consistent_hash",
            BalancingStrategy::CacheAware => "cache_aware",
            BalancingStrategy::Weighted => "weighted",
            BalancingStrategy::Pipeline => "pipeline",
            BalancingStrategy::Cheapest => "cheapest",
            BalancingStrategy::Fastest => "fastest",
            BalancingStrategy::PreciseCacheAware => "precise_cache_aware",
            BalancingStrategy::LmcacheAware => "lmcache_aware",
            BalancingStrategy::Adaptive => "adaptive",
        };
        let existing = routes
            .list(project_id)
            .await?
            .into_iter()
            .find(|row| row.model == r.model);
        let route_row = match existing {
            Some(row) => row,
            None => routes.create(project_id, &r.model, strategy).await?,
        };

        // round-trip the admin param defaults + override policy (idempotent:
        // re-importing overwrites with the toml's current values)
        if !r.params.is_empty()
            || r.param_policy.mode != rolter_core::OverrideMode::Allow
            || !r.param_policy.allow.is_empty()
            || !r.param_policy.deny.is_empty()
        {
            let params = serde_json::to_value(&r.params)?;
            let param_policy = serde_json::to_value(&r.param_policy)?;
            routes
                .set_params(route_row.id, &params, &param_policy)
                .await?;
        }

        let existing_targets = targets.list(route_row.id).await?;
        for t in &r.targets {
            let Some(&provider_id) = provider_ids.get(&t.provider) else {
                tracing::warn!(
                    target_provider = %t.provider,
                    model = %r.model,
                    "skipping target: provider not imported"
                );
                continue;
            };
            let already_imported = existing_targets.iter().any(|row| {
                row.provider_id == provider_id
                    && row.upstream_model.as_deref() == t.model.as_deref()
            });
            if already_imported {
                continue;
            }
            targets
                .create(
                    route_row.id,
                    provider_id,
                    t.model.as_deref(),
                    t.weight as i32,
                )
                .await?;
        }
        tracing::info!(model = %r.model, "imported route");
    }
    import_prompt_templates(pool, org_id, project_id, &config).await?;

    Ok(())
}

async fn import_prompt_templates(
    pool: &PgPool,
    org_id: Uuid,
    project_id: Uuid,
    config: &GatewayConfig,
) -> anyhow::Result<()> {
    if config.prompt_templates.templates.is_empty() {
        return Ok(());
    }

    let routes = RouteRepo(pool);
    let route_rows = routes.list(project_id).await?;
    let mut route_ids_by_model: HashMap<String, Vec<Uuid>> = HashMap::new();
    for route in route_rows {
        route_ids_by_model
            .entry(route.model.clone())
            .or_default()
            .push(route.id);
    }

    for template in &config.prompt_templates.templates {
        import_prompt_template(pool, org_id, template, &route_ids_by_model).await?;
    }
    Ok(())
}

async fn import_prompt_template(
    pool: &PgPool,
    org_id: Uuid,
    template: &PromptTemplate,
    route_ids_by_model: &HashMap<String, Vec<Uuid>>,
) -> anyhow::Result<()> {
    let variables = serde_json::to_value(&template.variables)?;
    let decorators = serde_json::to_value(&template.decorators)?;
    let version = i32::try_from(template.version)
        .map_err(|_| anyhow::anyhow!("prompt template version out of range"))?;

    let mut tx = pool.begin().await?;
    sqlx::query(
        "insert into prompt_templates (org_id, name, slug, description)
         values ($1, $2, $3, '')
         on conflict (org_id, slug) do nothing",
    )
    .bind(org_id)
    .bind(&template.id)
    .bind(&template.id)
    .execute(&mut *tx)
    .await?;
    let template_id: Uuid =
        sqlx::query_scalar("select id from prompt_templates where org_id = $1 and slug = $2")
            .bind(org_id)
            .bind(&template.id)
            .fetch_one(&mut *tx)
            .await?;

    let existing_version: Option<(serde_json::Value, serde_json::Value)> = sqlx::query_as(
        "select variables, decorators
         from prompt_template_versions
         where template_id = $1 and version = $2",
    )
    .bind(template_id)
    .bind(version)
    .fetch_optional(&mut *tx)
    .await?;
    match existing_version {
        Some((stored_variables, stored_decorators))
            if stored_variables != variables || stored_decorators != decorators =>
        {
            anyhow::bail!(
                "prompt template '{}' version {} differs from the immutable stored version",
                template.id,
                template.version
            );
        }
        Some(_) => {}
        None => {
            sqlx::query(
                "insert into prompt_template_versions (
                     template_id, version, variables, decorators
                 )
                 values ($1, $2, $3, $4)",
            )
            .bind(template_id)
            .bind(version)
            .bind(&variables)
            .bind(&decorators)
            .execute(&mut *tx)
            .await?;
        }
    }

    let published_version: Option<i32> =
        sqlx::query_scalar("select published_version from prompt_templates where id = $1")
            .bind(template_id)
            .fetch_one(&mut *tx)
            .await?;
    if published_version.is_none() {
        if template.routes.is_empty() {
            sqlx::query(
                "insert into prompt_template_scopes (
                     template_id, version, scope_type, scope_id, org_id
                 )
                 values ($1, $2, 'org', $3, $3)
                 on conflict (template_id, version, scope_type, scope_id) do nothing",
            )
            .bind(template_id)
            .bind(version)
            .bind(org_id)
            .execute(&mut *tx)
            .await?;
        } else {
            for model in &template.routes {
                let Some(route_ids) = route_ids_by_model.get(model) else {
                    tracing::warn!(template = %template.id, model, "skipping prompt-template route scope: route model not imported");
                    continue;
                };
                for route_id in route_ids {
                    sqlx::query(
                        "insert into prompt_template_scopes (
                             template_id, version, scope_type, scope_id, route_id
                         )
                         values ($1, $2, 'route', $3, $3)
                         on conflict (template_id, version, scope_type, scope_id) do nothing",
                    )
                    .bind(template_id)
                    .bind(version)
                    .bind(route_id)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }

        sqlx::query(
            "update prompt_templates
             set published_version = $2
             where id = $1",
        )
        .bind(template_id)
        .bind(version)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    tracing::info!(template = %template.id, version = template.version, "imported prompt template");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{import_prompt_template, slugify};
    use rolter_core::{
        Decorator, DecoratorPosition, DecoratorRole, PromptTemplate, TemplateVariable,
    };
    use std::collections::HashMap;
    use uuid::Uuid;

    #[test]
    fn slugify_normalizes() {
        assert_eq!(slugify("Default"), "default");
        assert_eq!(slugify("Acme Corp!"), "acme-corp");
        assert_eq!(slugify("  multi  space "), "multi-space");
    }

    #[tokio::test]
    async fn prompt_template_seed_is_idempotent_and_rejects_version_drift() {
        let Ok(url) = std::env::var("ROLTER_TEST_DATABASE_URL") else {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        };
        let schema = format!("seed_test_{}", Uuid::new_v4().simple());
        let admin = rolter_store::postgres::connect(&url).await.unwrap();
        sqlx::query(&format!("create schema {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let separator = if url.contains('?') { '&' } else { '?' };
        let scoped_url = format!("{url}{separator}options=-c%20search_path%3D{schema}");
        let pool = rolter_store::postgres::connect(&scoped_url).await.unwrap();
        rolter_store::postgres::run_migrations(&pool).await.unwrap();
        let org_id: Uuid = sqlx::query_scalar(
            "insert into orgs (name, slug) values ('acme', 'acme') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let template = PromptTemplate {
            id: "support-baseline".to_string(),
            version: 1,
            routes: Vec::new(),
            scopes: Vec::new(),
            variables: vec![TemplateVariable {
                name: "tone".to_string(),
                required: false,
                default: Some("calm".to_string()),
            }],
            decorators: vec![Decorator {
                role: DecoratorRole::System,
                position: DecoratorPosition::Prepend,
                content: "tone={{ tone }}".to_string(),
            }],
        };
        let route_ids = HashMap::new();
        import_prompt_template(&pool, org_id, &template, &route_ids)
            .await
            .unwrap();
        import_prompt_template(&pool, org_id, &template, &route_ids)
            .await
            .unwrap();

        let template_count: i64 = sqlx::query_scalar("select count(*) from prompt_templates")
            .fetch_one(&pool)
            .await
            .unwrap();
        let version_count: i64 =
            sqlx::query_scalar("select count(*) from prompt_template_versions")
                .fetch_one(&pool)
                .await
                .unwrap();
        let scope_count: i64 = sqlx::query_scalar("select count(*) from prompt_template_scopes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((template_count, version_count, scope_count), (1, 1, 1));

        let mut drifted = template;
        drifted.decorators[0].content = "different".to_string();
        assert!(import_prompt_template(&pool, org_id, &drifted, &route_ids)
            .await
            .is_err());
    }
}
