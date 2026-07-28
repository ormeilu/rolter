//! End-to-end control-plane integration tests against a real Postgres.
//!
//! Gated on the `postgres` feature and the `ROLTER_TEST_DATABASE_URL` env var:
//! when the var is unset (local runs without a database) the tests self-skip.
//! CI provides a Postgres service and the var, so they run there.
#![cfg(feature = "postgres")]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{json, Value};

fn database_url() -> Option<String> {
    std::env::var("ROLTER_TEST_DATABASE_URL").ok()
}

/// Monotonic sequence for per-test schema names, keeping concurrently-running
/// tests off a shared schema (plain `cargo test` — e.g. the coverage job —
/// runs them as threads in one process, so a shared `public` races on DDL).
static SCHEMA_SEQ: AtomicU32 = AtomicU32::new(0);

/// Isolated schema name unique to this process and call, safe to interpolate
/// (only ascii digits and underscores).
fn unique_schema() -> String {
    let n = SCHEMA_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("test_{}_{}", std::process::id(), n)
}

/// Return `url` with the connection pinned to `schema` via `search_path`, so
/// migrations and queries land in the isolated schema rather than `public`.
fn with_search_path(url: &str, schema: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    // percent-encode the space and `=` inside the libpq options string
    format!("{url}{sep}options=-c%20search_path%3D{schema}")
}

/// Create a fresh isolated schema and return a pool pinned to it. Tests that
/// only need a router use [`fresh_app`]; those that also query the store
/// directly build the router from this pool themselves.
async fn fresh_pool() -> sqlx::PgPool {
    let url = database_url().expect("ROLTER_TEST_DATABASE_URL checked by caller");
    let schema = unique_schema();

    // (re)create the isolated schema over a default-search_path connection
    let admin = rolter_store::postgres::connect(&url)
        .await
        .expect("connect");
    sqlx::query(&format!("drop schema if exists {schema} cascade"))
        .execute(&admin)
        .await
        .expect("reset schema");
    sqlx::query(&format!("create schema {schema}"))
        .execute(&admin)
        .await
        .expect("recreate schema");
    admin.close().await;

    // app pool scoped to the isolated schema so migrations run there
    rolter_store::postgres::connect(&with_search_path(&url, &schema))
        .await
        .expect("connect scoped")
}

/// Isolated schema + control-plane app router (migrations applied by `test_app`).
async fn fresh_app() -> axum::Router {
    let pool = fresh_pool().await;
    rolter_control::test_app(pool).await.expect("build app")
}

/// Serve `app` on an ephemeral port and return its address.
async fn serve(app: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

macro_rules! skip_without_db {
    () => {
        if database_url().is_none() {
            eprintln!("skipping: ROLTER_TEST_DATABASE_URL not set");
            return;
        }
    };
}

#[tokio::test]
async fn ping_and_healthz_respond() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);

    let ping: Value = client
        .get(format!("http://{addr}/api/v1/ping"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ping["pong"], true);
}

#[tokio::test]
async fn snapshot_served_on_empty_store() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/internal/snapshot"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["version"].is_number(),
        "snapshot missing version: {body}"
    );
    assert!(
        body["config"].is_object(),
        "snapshot missing config: {body}"
    );
}

#[tokio::test]
async fn crud_create_round_trip_reflects_in_snapshot() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    // helper: POST json, assert 2xx, return the parsed body
    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    // org → team → project hierarchy
    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().expect("org id");

    let team = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Platform"}),
    )
    .await;
    let team_id = team["id"].as_str().expect("team id");

    let project = post(
        &client,
        format!("{base}/api/v1/teams/{team_id}/projects"),
        json!({"name": "Gateway"}),
    )
    .await;
    let project_id = project["id"].as_str().expect("project id");

    // provider under the org
    let provider = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/providers"),
        json!({"name": "openai", "kind": "openai", "api_base": "https://api.openai.com"}),
    )
    .await;
    let provider_id = provider["id"].as_str().expect("provider id");

    // route under the project, plus a target pointing at the provider
    let route = post(
        &client,
        format!("{base}/api/v1/projects/{project_id}/routes"),
        json!({"model": "gpt-4o", "strategy": "round_robin"}),
    )
    .await;
    let route_id = route["id"].as_str().expect("route id");

    let advanced = client
        .put(format!("{base}/api/v1/routes/{route_id}/advanced"))
        .json(&json!({
            "advanced": {
                "model_type": "chat",
                "capabilities": ["tools", "vision"],
                "description": "managed model",
                "base_url": "https://models.example/v1",
                "pricing": {"image_per_unit": 0.04},
                "limits": {"output_tokens": 2048, "timeout_secs": 30},
                "headers": {"x-model-region": "eu"},
                "locked_headers": ["x-model-region"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(advanced.status().is_success());

    post(
        &client,
        format!("{base}/api/v1/routes/{route_id}/targets"),
        json!({"provider_id": provider_id, "weight": 1}),
    )
    .await;

    // the snapshot the gateway polls must now reflect the new provider + route
    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(
        snap["version"].as_u64().unwrap_or(0) > 0,
        "version should bump after writes: {snap}"
    );
    let providers = snap["config"]["providers"].as_array().expect("providers");
    assert!(
        providers.iter().any(|p| p["name"] == "openai"),
        "provider missing from snapshot: {snap}"
    );
    let routes = snap["config"]["routes"].as_array().expect("routes");
    assert!(
        routes.iter().any(|r| r["model"] == "gpt-4o"),
        "route missing from snapshot: {snap}"
    );
    let route = routes
        .iter()
        .find(|r| r["model"] == "gpt-4o")
        .expect("route in snapshot");
    assert_eq!(route["advanced"]["limits"]["output_tokens"], 2048);
    assert_eq!(route["advanced"]["headers"]["x-model-region"], "eu");
}

#[tokio::test]
async fn business_unit_and_customer_crud_round_trip() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().expect("org id");

    let other_org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Other", "slug": "other"}),
    )
    .await;
    let other_org_id = other_org["id"].as_str().expect("other org id");

    let business_unit = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/business-units"),
        json!({"name": "Payments"}),
    )
    .await;
    let business_unit_id = business_unit["id"].as_str().expect("business unit id");
    assert_eq!(business_unit["slug"], "payments");

    let customer = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/customers"),
        json!({"name": "Acme EU", "business_unit_id": business_unit_id}),
    )
    .await;
    let customer_id = customer["id"].as_str().expect("customer id");
    assert_eq!(customer["slug"], "acme-eu");
    assert_eq!(customer["business_unit_id"], business_unit_id);

    let mismatch_unit = post(
        &client,
        format!("{base}/api/v1/orgs/{other_org_id}/business-units"),
        json!({"name": "Other Unit"}),
    )
    .await;
    let mismatch_unit_id = mismatch_unit["id"].as_str().expect("mismatch unit id");
    let invalid_customer = client
        .post(format!("{base}/api/v1/orgs/{org_id}/customers"))
        .json(&json!({"name": "Broken Link", "business_unit_id": mismatch_unit_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_customer.status(), 400);

    let update_customer = client
        .put(format!("{base}/api/v1/customers/{customer_id}"))
        .json(&json!({
            "slug": "acme-emea",
            "allow_slug_change": true,
            "retired": true
        }))
        .send()
        .await
        .unwrap();
    assert!(update_customer.status().is_success());
    let updated_customer: Value = update_customer.json().await.unwrap();
    assert_eq!(updated_customer["slug"], "acme-emea");
    assert!(updated_customer["retired_at"].is_string());

    let list_customers: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/customers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list_customers.as_array().unwrap().len(), 1);

    let clear_business_unit = client
        .put(format!("{base}/api/v1/customers/{customer_id}"))
        .json(&json!({ "business_unit_id": null, "retired": false }))
        .send()
        .await
        .unwrap();
    assert!(clear_business_unit.status().is_success());
    let cleared_customer: Value = clear_business_unit.json().await.unwrap();
    assert!(cleared_customer["business_unit_id"].is_null());
    assert!(cleared_customer["retired_at"].is_null());

    let retire_business_unit = client
        .put(format!("{base}/api/v1/business-units/{business_unit_id}"))
        .json(&json!({"retired": true}))
        .send()
        .await
        .unwrap();
    assert!(retire_business_unit.status().is_success());
    let updated_unit: Value = retire_business_unit.json().await.unwrap();
    assert!(updated_unit["retired_at"].is_string());

    let delete_customer = client
        .delete(format!("{base}/api/v1/customers/{customer_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_customer.status(), 204);

    let delete_business_unit = client
        .delete(format!("{base}/api/v1/business-units/{business_unit_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_business_unit.status(), 204);
}

#[tokio::test]
async fn governance_scoped_budgets_and_rate_limits() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().expect("org id");

    let unit = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/business-units"),
        json!({"name": "Payments"}),
    )
    .await;
    let unit_id = unit["id"].as_str().expect("business unit id");

    let customer = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/customers"),
        json!({"name": "Acme EU"}),
    )
    .await;
    let customer_id = customer["id"].as_str().expect("customer id");

    let budget = post(
        &client,
        format!("{base}/api/v1/budgets"),
        json!({"scope_type": "business_unit", "scope_id": unit_id, "limit_usd": "250.0"}),
    )
    .await;
    assert_eq!(budget["scope_type"], "business_unit");

    let limit = post(
        &client,
        format!("{base}/api/v1/rate-limits"),
        json!({"scope_type": "customer", "scope_id": customer_id, "rpm": 60}),
    )
    .await;
    assert_eq!(limit["scope_type"], "customer");

    let listed: Value = client
        .get(format!(
            "{base}/api/v1/budgets?scope_type=business_unit&scope_id={unit_id}"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    // an unknown scope type is rejected before it reaches the store
    let bad = client
        .post(format!("{base}/api/v1/budgets"))
        .json(&json!({"scope_type": "division", "scope_id": unit_id, "limit_usd": "1.0"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    // the gateway snapshot carries both caps with their governance scopes
    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let budgets = snap["config"]["budgets"].as_array().expect("budgets");
    assert!(budgets
        .iter()
        .any(|b| b["scope"] == "business_unit" && b["id"] == unit_id));
    let limits = snap["config"]["rate_limits"].as_array().expect("limits");
    assert!(limits
        .iter()
        .any(|l| l["scope"] == "customer" && l["id"] == customer_id));
}

#[tokio::test]
async fn virtual_key_cost_attribution_round_trip() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().expect("org id");

    let team = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Platform"}),
    )
    .await;
    let team_id = team["id"].as_str().expect("team id");

    let project = post(
        &client,
        format!("{base}/api/v1/teams/{team_id}/projects"),
        json!({"name": "Gateway"}),
    )
    .await;
    let project_id = project["id"].as_str().expect("project id");

    let virtual_key = post(
        &client,
        format!("{base}/api/v1/projects/{project_id}/virtual-keys"),
        json!({"name": "billing-key"}),
    )
    .await;
    let virtual_key_id = virtual_key["id"].as_str().expect("virtual key id");
    assert!(virtual_key["business_unit_id"].is_null());
    assert!(virtual_key["customer_id"].is_null());

    let unit = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/business-units"),
        json!({"name": "Payments"}),
    )
    .await;
    let unit_id = unit["id"].as_str().expect("business unit id");

    let customer = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/customers"),
        json!({"name": "Acme EU", "business_unit_id": unit_id}),
    )
    .await;
    let customer_id = customer["id"].as_str().expect("customer id");

    let attributed: Value = client
        .put(format!(
            "{base}/api/v1/virtual-keys/{virtual_key_id}/attribution"
        ))
        .json(&json!({"business_unit_id": unit_id, "customer_id": customer_id}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(attributed["business_unit_id"], unit_id);
    assert_eq!(attributed["customer_id"], customer_id);

    // the gateway snapshot carries the attribution so usage can be tagged
    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let keys = snap["config"]["db_virtual_keys"]
        .as_array()
        .expect("db_virtual_keys");
    let key = keys
        .iter()
        .find(|k| k["id"] == virtual_key_id)
        .expect("virtual key in snapshot");
    assert_eq!(key["business_unit_id"], unit_id);
    assert_eq!(key["customer_id"], customer_id);

    // a customer from another org may not be attached
    let other_org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Other", "slug": "other"}),
    )
    .await;
    let other_org_id = other_org["id"].as_str().expect("other org id");
    let foreign_customer = post(
        &client,
        format!("{base}/api/v1/orgs/{other_org_id}/customers"),
        json!({"name": "Foreign"}),
    )
    .await;
    let foreign_customer_id = foreign_customer["id"]
        .as_str()
        .expect("foreign customer id");
    let cross_org = client
        .put(format!(
            "{base}/api/v1/virtual-keys/{virtual_key_id}/attribution"
        ))
        .json(&json!({"customer_id": foreign_customer_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_org.status(), 400);

    // pairing a customer with a business unit that does not own it is rejected
    let other_unit = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/business-units"),
        json!({"name": "Growth"}),
    )
    .await;
    let other_unit_id = other_unit["id"].as_str().expect("other unit id");
    let mismatched = client
        .put(format!(
            "{base}/api/v1/virtual-keys/{virtual_key_id}/attribution"
        ))
        .json(&json!({"business_unit_id": other_unit_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(mismatched.status(), 400);

    // omitted fields stay put; explicit nulls clear the dimension
    let cleared: Value = client
        .put(format!(
            "{base}/api/v1/virtual-keys/{virtual_key_id}/attribution"
        ))
        .json(&json!({"customer_id": null}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cleared["business_unit_id"], unit_id);
    assert!(cleared["customer_id"].is_null());

    // deleting the unit detaches the keys that pointed at it
    let delete_customer = client
        .delete(format!("{base}/api/v1/customers/{customer_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_customer.status(), 204);
    let delete_unit = client
        .delete(format!("{base}/api/v1/business-units/{unit_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_unit.status(), 204);
    let orphaned: Value = client
        .get(format!("{base}/api/v1/projects/{project_id}/virtual-keys"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(orphaned[0]["business_unit_id"].is_null());
}

#[tokio::test]
async fn prompt_template_crud_publish_and_scope_round_trip() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().expect("org id");

    let team = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Platform"}),
    )
    .await;
    let team_id = team["id"].as_str().expect("team id");

    let project = post(
        &client,
        format!("{base}/api/v1/teams/{team_id}/projects"),
        json!({"name": "Gateway"}),
    )
    .await;
    let project_id = project["id"].as_str().expect("project id");

    let provider = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/providers"),
        json!({"name": "openai", "kind": "openai", "api_base": "https://api.openai.com"}),
    )
    .await;
    let provider_id = provider["id"].as_str().expect("provider id");

    let route = post(
        &client,
        format!("{base}/api/v1/projects/{project_id}/routes"),
        json!({"model": "gpt-4o-mini", "strategy": "round_robin"}),
    )
    .await;
    let route_id = route["id"].as_str().expect("route id");

    let virtual_key = post(
        &client,
        format!("{base}/api/v1/projects/{project_id}/virtual-keys"),
        json!({"name": "template-key", "models": ["gpt-4o-mini"], "providers": [provider_id]}),
    )
    .await;
    let virtual_key_id = virtual_key["id"].as_str().expect("virtual key id");

    let template = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/prompt-templates"),
        json!({"name": "support baseline", "description": "v1"}),
    )
    .await;
    let template_id = template["id"].as_str().expect("template id");
    assert_eq!(template["slug"], "support-baseline");

    let templates: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/prompt-templates"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(templates.as_array().unwrap().len(), 1);

    let version1 = post(
        &client,
        format!("{base}/api/v1/prompt-templates/{template_id}/versions"),
        json!({
            "variables": [{"name": "tone", "required": true}],
            "decorators": [{"role": "system", "position": "prepend", "content": "tone={{ tone }}"}]
        }),
    )
    .await;
    assert_eq!(version1["version"], 1);

    let version2 = post(
        &client,
        format!("{base}/api/v1/prompt-templates/{template_id}/versions"),
        json!({"variables": [], "decorators": []}),
    )
    .await;
    assert_eq!(version2["version"], 2);

    let set_scopes = client
        .put(format!(
            "{base}/api/v1/prompt-templates/{template_id}/versions/2/scopes"
        ))
        .json(&json!({
            "scopes": [
                {"scope_type": "org", "scope_id": org_id},
                {"scope_type": "project", "scope_id": project_id},
                {"scope_type": "route", "scope_id": route_id},
                {"scope_type": "virtual_key", "scope_id": virtual_key_id}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(set_scopes.status(), 204);

    let version1_scopes = client
        .put(format!(
            "{base}/api/v1/prompt-templates/{template_id}/versions/1/scopes"
        ))
        .json(&json!({
            "scopes": [{"scope_type": "org", "scope_id": org_id}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(version1_scopes.status(), 204);

    let publish = client
        .put(format!(
            "{base}/api/v1/prompt-templates/{template_id}/publish"
        ))
        .json(&json!({"version": 2}))
        .send()
        .await
        .unwrap();
    let publish_status = publish.status();
    let published: Value = publish.json().await.unwrap();
    assert!(
        publish_status.is_success(),
        "publish failed ({publish_status}): {published}"
    );
    assert_eq!(published["published_version"], 2);

    let mutate_published = client
        .put(format!(
            "{base}/api/v1/prompt-templates/{template_id}/versions/2/scopes"
        ))
        .json(&json!({
            "scopes": [{"scope_type": "org", "scope_id": org_id}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mutate_published.status(), 400);

    let scopes: Value = client
        .get(format!(
            "{base}/api/v1/prompt-templates/{template_id}/versions/2/scopes"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(scopes.as_array().unwrap().len(), 4);

    let other_org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Other", "slug": "other"}),
    )
    .await;
    let other_org_id = other_org["id"].as_str().expect("other org id");
    let other_team = post(
        &client,
        format!("{base}/api/v1/orgs/{other_org_id}/teams"),
        json!({"name": "Other Team"}),
    )
    .await;
    let other_team_id = other_team["id"].as_str().expect("other team id");
    let other_project = post(
        &client,
        format!("{base}/api/v1/teams/{other_team_id}/projects"),
        json!({"name": "Other Project"}),
    )
    .await;
    let other_project_id = other_project["id"].as_str().expect("other project id");
    let bad_scope = client
        .put(format!(
            "{base}/api/v1/prompt-templates/{template_id}/versions/2/scopes"
        ))
        .json(&json!({
            "scopes": [
                {"scope_type": "project", "scope_id": other_project_id}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_scope.status(), 400);

    let rollback = client
        .put(format!(
            "{base}/api/v1/prompt-templates/{template_id}/rollback"
        ))
        .json(&json!({"version": 1}))
        .send()
        .await
        .unwrap();
    let rollback_status = rollback.status();
    let rolled_back: Value = rollback.json().await.unwrap();
    assert!(
        rollback_status.is_success(),
        "rollback failed ({rollback_status}): {rolled_back}"
    );
    assert_eq!(rolled_back["published_version"], 1);

    let delete_template = client
        .delete(format!("{base}/api/v1/prompt-templates/{template_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_template.status(), 204);
}

#[tokio::test]
async fn skills_crud_and_publish_round_trip() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().expect("org id");
    let team = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Skills Team"}),
    )
    .await;
    let team_id = team["id"].as_str().expect("team id");

    let skill = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/skills"),
        json!({
            "name": "classification baseline",
            "description": "v1",
            "allowed_team_ids": [team_id],
            "minimum_role": "viewer"
        }),
    )
    .await;
    let skill_id = skill["id"].as_str().expect("skill id");
    assert_eq!(skill["slug"], "classification-baseline");

    let skills: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/skills"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(skills.as_array().unwrap().len(), 1);

    let v1 = post(
        &client,
        format!("{base}/api/v1/skills/{skill_id}/versions"),
        json!({"content": "alpha", "metadata": {"author": "ops"}}),
    )
    .await;
    assert_eq!(v1["version"], 1);

    let v2 = post(
        &client,
        format!("{base}/api/v1/skills/{skill_id}/versions"),
        json!({
            "content_ref": "oci://registry.example/skills/classification@sha256:abc",
            "metadata": {"author": "ops"}
        }),
    )
    .await;
    assert_eq!(v2["version"], 2);
    assert!(v2["content"].is_null());
    assert!(v2["content_ref"].is_string());

    let rejected_secret_metadata = client
        .post(format!("{base}/api/v1/skills/{skill_id}/versions"))
        .json(&json!({
            "content": "gamma",
            "metadata": {"api_token": "must-not-be-stored"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected_secret_metadata.status(), 400);

    let versions: Value = client
        .get(format!("{base}/api/v1/skills/{skill_id}/versions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(versions.as_array().unwrap().len(), 2);

    let publish = client
        .put(format!("{base}/api/v1/skills/{skill_id}/publish"))
        .json(&json!({"version": 2}))
        .send()
        .await
        .unwrap();
    let publish_status = publish.status();
    let published: Value = publish.json().await.unwrap();
    assert!(
        publish_status.is_success(),
        "publish failed ({publish_status}): {published}"
    );
    assert_eq!(published["published_version"], 2);

    let resolved: Value = client
        .get(format!(
            "{base}/api/v1/orgs/{org_id}/skills/resolve/classification-baseline"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved["version"], 2);
    assert!(resolved["content"].is_null());
    assert_eq!(
        resolved["content_ref"],
        "oci://registry.example/skills/classification@sha256:abc"
    );

    let rollback = client
        .put(format!("{base}/api/v1/skills/{skill_id}/rollback"))
        .json(&json!({"version": 1}))
        .send()
        .await
        .unwrap();
    let rollback_status = rollback.status();
    let rolled_back: Value = rollback.json().await.unwrap();
    assert!(
        rollback_status.is_success(),
        "rollback failed ({rollback_status}): {rolled_back}"
    );
    assert_eq!(rolled_back["published_version"], 1);

    let retire = client
        .put(format!("{base}/api/v1/skills/{skill_id}"))
        .json(&json!({"retired": true}))
        .send()
        .await
        .unwrap();
    assert!(retire.status().is_success());
    let retired: Value = retire.json().await.unwrap();
    assert!(retired["retired_at"].is_string());

    let retired_resolution = client
        .get(format!(
            "{base}/api/v1/orgs/{org_id}/skills/resolve/classification-baseline"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(retired_resolution.status(), 404);

    let delete_skill = client
        .delete(format!("{base}/api/v1/skills/{skill_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_skill.status(), 204);
}

/// Provider credentials posted to the API must be sealed at rest, decrypted
/// into the gateway snapshot, and never leak through the dashboard config
/// endpoint. Runs in its own process (nextest), so setting the KEK env var
/// here cannot race other tests.
#[tokio::test]
async fn provider_api_key_seals_at_rest_and_decrypts_into_snapshot() {
    skip_without_db!();
    std::env::set_var("ROLTER_KEK", "integration-test-kek");

    let pool = fresh_pool().await;
    let app = rolter_control::test_app(pool.clone()).await.unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let org: Value = client
        .post(format!("{base}/api/v1/orgs"))
        .json(&json!({"name": "Acme", "slug": "acme"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let org_id = org["id"].as_str().expect("org id");

    let provider: Value = client
        .post(format!("{base}/api/v1/orgs/{org_id}/providers"))
        .json(&json!({
            "name": "openai",
            "kind": "openai",
            "api_base": "https://api.openai.com",
            "api_key": "sk-live-secret",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = provider["id"].as_str().expect("provider id");

    // at rest: sealed, not plaintext
    let ciphertext: Vec<u8> =
        sqlx::query_scalar("select ciphertext from provider_keys where provider_id = $1::uuid")
            .bind(provider_id)
            .fetch_one(&pool)
            .await
            .expect("provider_keys row must exist");
    assert!(
        !String::from_utf8_lossy(&ciphertext).contains("sk-live-secret"),
        "credential must not be stored in plaintext"
    );

    // gateway snapshot: decrypted and usable
    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        snap["config"]["providers"][0]["api_key"], "sk-live-secret",
        "snapshot must carry the decrypted key: {snap}"
    );

    // dashboard config: redacted
    let config_body = client
        .get(format!("{base}/api/v1/config"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        !config_body.contains("sk-live-secret"),
        "config endpoint must redact provider keys"
    );

    // rotate via PUT, then clear with an empty string
    let updated = client
        .put(format!("{base}/api/v1/providers/{provider_id}"))
        .json(&json!({"api_key": "sk-rotated", "api_base": "https://eu.api.openai.com"}))
        .send()
        .await
        .unwrap();
    assert!(updated.status().is_success(), "{}", updated.status());

    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snap["config"]["providers"][0]["api_key"], "sk-rotated");
    assert_eq!(
        snap["config"]["providers"][0]["api_base"],
        "https://eu.api.openai.com"
    );

    let cleared = client
        .put(format!("{base}/api/v1/providers/{provider_id}"))
        .json(&json!({"api_key": ""}))
        .send()
        .await
        .unwrap();
    assert!(cleared.status().is_success());

    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        snap["config"]["providers"][0]["api_key"].is_null(),
        "cleared key must drop from the snapshot: {snap}"
    );
}

/// With an admin token configured, the CRUD API and snapshot endpoint reject
/// unauthenticated calls and accept the bearer token.
#[tokio::test]
async fn admin_token_guards_crud_and_snapshot() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool, Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let denied = client
        .post(format!("{base}/api/v1/orgs"))
        .json(&json!({"name": "Acme", "slug": "acme"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    let denied_snapshot = client
        .get(format!("{base}/internal/snapshot"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_snapshot.status(), 401);

    let allowed = client
        .post(format!("{base}/api/v1/orgs"))
        .bearer_auth("sekrit")
        .json(&json!({"name": "Acme", "slug": "acme"}))
        .send()
        .await
        .unwrap();
    assert!(allowed.status().is_success(), "{}", allowed.status());
}

/// Persisted feature flags are superadmin-only, survive reads, and write an
/// audit entry for each update.
#[tokio::test]
async fn feature_flags_are_superadmin_only_and_audited() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let denied = client
        .get(format!("{base}/api/v1/feature-flags"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    let baseline: Value = client
        .get(format!("{base}/api/v1/feature-flags"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(baseline["response_cache"], true);
    assert_eq!(baseline["cache_aware_routing"], true);
    assert_eq!(baseline["circuit_breaker"], true);
    assert_eq!(baseline["active_health_checks"], true);
    assert_eq!(baseline["complexity_routing"], true);
    assert_eq!(baseline["guardrails"], true);

    let updated: Value = client
        .put(format!("{base}/api/v1/feature-flags"))
        .bearer_auth("sekrit")
        .json(&json!({
            "response_cache": false,
            "cache_aware_routing": false,
            "circuit_breaker": false,
            "active_health_checks": false,
            "complexity_routing": false,
            "guardrails": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["response_cache"], false);
    assert_eq!(updated["cache_aware_routing"], false);
    assert_eq!(updated["circuit_breaker"], false);
    assert_eq!(updated["active_health_checks"], false);
    assert_eq!(updated["complexity_routing"], false);
    assert_eq!(updated["guardrails"], false);

    let reloaded: Value = client
        .get(format!("{base}/api/v1/feature-flags"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reloaded["response_cache"], false);

    let action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'feature_flags.update' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(action.as_deref(), Some("feature_flags.update"));
}

#[tokio::test]
async fn cluster_inventory_tracks_polling_nodes() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    // an anonymous poll stays out of the inventory: single-node deployments
    // need no cluster setup
    let anonymous = client
        .get(format!("{base}/internal/snapshot"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap();
    assert!(anonymous.status().is_success());
    let nodes: Value = client
        .get(format!("{base}/api/v1/cluster/nodes"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(nodes.as_array().unwrap().len(), 0);

    // an identified poll registers the node and its applied config version
    let identified = client
        .get(format!("{base}/internal/snapshot?version=0"))
        .bearer_auth("sekrit")
        .header("x-rolter-node-id", "gw-1")
        .header("x-rolter-node-role", "gateway")
        .header("x-rolter-node-build", "0.0.10")
        .send()
        .await
        .unwrap();
    assert!(identified.status().is_success());

    let nodes: Value = client
        .get(format!("{base}/api/v1/cluster/nodes"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let node = &nodes.as_array().expect("nodes")[0];
    assert_eq!(node["id"], "gw-1");
    assert_eq!(node["role"], "gateway");
    assert_eq!(node["build_version"], "0.0.10");
    assert_eq!(node["live"], true);

    // the inventory is superadmin-only
    let denied = client
        .get(format!("{base}/api/v1/cluster/nodes"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    // the only live gateway may not be drained: it would take the data plane down
    let refused = client
        .put(format!("{base}/api/v1/cluster/nodes/gw-1/drain"))
        .bearer_auth("sekrit")
        .json(&json!({"draining": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 400);

    // with a second live gateway the drain is safe, and the draining node
    // learns about it on its next poll
    let second = client
        .get(format!("{base}/internal/snapshot?version=0"))
        .bearer_auth("sekrit")
        .header("x-rolter-node-id", "gw-2")
        .header("x-rolter-node-role", "gateway")
        .send()
        .await
        .unwrap();
    assert!(second.status().is_success());

    let drained: Value = client
        .put(format!("{base}/api/v1/cluster/nodes/gw-1/drain"))
        .bearer_auth("sekrit")
        .json(&json!({"draining": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(drained["desired_state"], "draining");

    let polled = client
        .get(format!("{base}/internal/snapshot?version=0"))
        .bearer_auth("sekrit")
        .header("x-rolter-node-id", "gw-1")
        .header("x-rolter-node-role", "gateway")
        .send()
        .await
        .unwrap();
    assert_eq!(
        polled
            .headers()
            .get("x-rolter-node-state")
            .and_then(|v| v.to_str().ok()),
        Some("draining")
    );

    let drain_action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'cluster_node.set_drain' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(drain_action.as_deref(), Some("cluster_node.set_drain"));

    // returning it to service is the same call with draining=false
    let restored: Value = client
        .put(format!("{base}/api/v1/cluster/nodes/gw-1/drain"))
        .bearer_auth("sekrit")
        .json(&json!({"draining": false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["desired_state"], "active");

    let forgotten_second = client
        .delete(format!("{base}/api/v1/cluster/nodes/gw-2"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap();
    assert_eq!(forgotten_second.status(), 204);

    // a decommissioned node can be forgotten, and the action is audited
    let forgotten = client
        .delete(format!("{base}/api/v1/cluster/nodes/gw-1"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap();
    assert_eq!(forgotten.status(), 204);
    let nodes: Value = client
        .get(format!("{base}/api/v1/cluster/nodes"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(nodes.as_array().unwrap().len(), 0);

    let action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'cluster_node.forget' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(action.as_deref(), Some("cluster_node.forget"));
}

#[tokio::test]
async fn unavailable_feature_flags_are_reported_and_cannot_be_enabled() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    // this deployment has no redis and no cache-publishing provider
    let view: Value = client
        .get(format!("{base}/api/v1/feature-flags"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let unavailable = view["unavailable"].as_array().expect("unavailable list");
    let names: Vec<&str> = unavailable
        .iter()
        .map(|u| u["flag"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"response_cache"), "{view}");
    assert!(names.contains(&"cache_aware_routing"), "{view}");
    assert!(unavailable
        .iter()
        .all(|u| !u["reason"].as_str().unwrap_or_default().is_empty()));

    // turning one on would persist a policy the gateway silently ignores
    let rejected = client
        .put(format!("{base}/api/v1/feature-flags"))
        .bearer_auth("sekrit")
        .json(&json!({
            "response_cache": true,
            "cache_aware_routing": false,
            "circuit_breaker": true,
            "active_health_checks": true,
            "complexity_routing": true,
            "guardrails": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);

    // the available flags stay editable
    let updated: Value = client
        .put(format!("{base}/api/v1/feature-flags"))
        .bearer_auth("sekrit")
        .json(&json!({
            "response_cache": false,
            "cache_aware_routing": false,
            "circuit_breaker": false,
            "active_health_checks": true,
            "complexity_routing": true,
            "guardrails": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["circuit_breaker"], false);
    assert_eq!(updated["unavailable"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn logging_settings_are_superadmin_only_and_audited() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let denied = client
        .get(format!("{base}/api/v1/logging-settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    let baseline: Value = client
        .get(format!("{base}/api/v1/logging-settings"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(baseline["sample_rate"], 1.0);

    let updated: Value = client
        .put(format!("{base}/api/v1/logging-settings"))
        .bearer_auth("sekrit")
        .json(&json!({
            "sample_rate": 0.25,
            "payload_capture_enabled": true,
            "payload_capture_max_bytes": 4096,
            "payload_capture_redact_fields": ["token", "authorization"],
            "payload_capture_models": ["gpt-4o"],
            "payload_capture_virtual_key_ids": ["00000000-0000-0000-0000-000000000001"],
            "retention_days": 30,
            "payload_retention_hours": 24
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["sample_rate"], 0.25);
    assert_eq!(updated["payload_capture_enabled"], true);
    assert_eq!(updated["payload_capture_max_bytes"], 4096);
    assert_eq!(updated["retention_days"], 30);
    assert_eq!(updated["payload_retention_hours"], 24);

    // raw bodies may never outlive the metadata row they belong to
    let rejected = client
        .put(format!("{base}/api/v1/logging-settings"))
        .bearer_auth("sekrit")
        .json(&json!({
            "sample_rate": 0.25,
            "payload_capture_enabled": true,
            "payload_capture_max_bytes": 4096,
            "payload_capture_redact_fields": ["token"],
            "payload_capture_models": [],
            "payload_capture_virtual_key_ids": [],
            "retention_days": 1,
            "payload_retention_hours": 720
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);

    let reloaded: Value = client
        .get(format!("{base}/api/v1/logging-settings"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reloaded["sample_rate"], 0.25);
    assert_eq!(reloaded["payload_capture_enabled"], true);
    assert_eq!(reloaded["retention_days"], 30);

    let action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'logging_settings.update' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(action.as_deref(), Some("logging_settings.update"));
}

#[tokio::test]
async fn runtime_policy_is_superadmin_only_and_audited() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let denied = client
        .get(format!("{base}/api/v1/runtime-policy"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    let baseline: Value = client
        .get(format!("{base}/api/v1/runtime-policy"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(baseline["retry_max_retries"], 2);
    assert_eq!(baseline["queue_backpressure"], "error");

    let updated: Value = client
        .put(format!("{base}/api/v1/runtime-policy"))
        .bearer_auth("sekrit")
        .json(&json!({
            "retry_max_retries": 4,
            "retry_base_ms": 150,
            "retry_max_ms": 1500,
            "timeout_connect_s": 20,
            "timeout_request_s": 180,
            "queue_enabled": true,
            "queue_capacity": 1024,
            "queue_workers": 16,
            "queue_backpressure": "block",
            "queue_block_ms": 5000
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["retry_max_retries"], 4);
    assert_eq!(updated["timeout_connect_s"], 20);
    assert_eq!(updated["queue_backpressure"], "block");

    let reloaded: Value = client
        .get(format!("{base}/api/v1/runtime-policy"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reloaded["retry_max_retries"], 4);
    assert_eq!(reloaded["queue_backpressure"], "block");

    let action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'runtime_policy.update' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(action.as_deref(), Some("runtime_policy.update"));
}

#[tokio::test]
async fn compatibility_policy_is_superadmin_only_validated_and_audited() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let denied = client
        .get(format!("{base}/api/v1/compatibility-policy"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    let baseline: Value = client
        .get(format!("{base}/api/v1/compatibility-policy"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // defaults preserve the previously compiled-in behavior
    assert_eq!(baseline["anthropic_version"], "2023-06-01");
    assert_eq!(baseline["default_max_tokens"], 1024);
    assert_eq!(baseline["restart_required"].as_array().unwrap().len(), 0);

    // a free-form version would fail every anthropic call at the edge
    let rejected = client
        .put(format!("{base}/api/v1/compatibility-policy"))
        .bearer_auth("sekrit")
        .json(&json!({"anthropic_version": "latest", "default_max_tokens": 1024}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);

    let updated: Value = client
        .put(format!("{base}/api/v1/compatibility-policy"))
        .bearer_auth("sekrit")
        .json(&json!({"anthropic_version": "2024-10-22", "default_max_tokens": 4096}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["anthropic_version"], "2024-10-22");
    assert_eq!(updated["default_max_tokens"], 4096);

    // the gateway snapshot carries the new policy without a restart
    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        snap["config"]["compatibility"]["anthropic_version"],
        "2024-10-22"
    );
    assert_eq!(snap["config"]["compatibility"]["default_max_tokens"], 4096);

    let action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'compatibility_policy.update' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(action.as_deref(), Some("compatibility_policy.update"));
}

#[tokio::test]
async fn adaptive_routing_policy_is_superadmin_only_validated_and_audited() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("sekrit".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let denied = client
        .get(format!("{base}/api/v1/adaptive-routing-policy"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    let baseline: Value = client
        .get(format!("{base}/api/v1/adaptive-routing-policy"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // shipped off, so enabling it is always a deliberate action
    assert_eq!(baseline["enabled"], false);
    assert_eq!(baseline["min_samples"], 50);
    assert_eq!(baseline["affected_routes"].as_array().unwrap().len(), 0);

    // an all-zero blend would make `adaptive` a random balancer
    let rejected = client
        .put(format!("{base}/api/v1/adaptive-routing-policy"))
        .bearer_auth("sekrit")
        .json(&json!({
            "enabled": true,
            "latency_weight": 0.0,
            "cost_weight": 0.0,
            "load_weight": 0.0,
            "exploration_ratio": 0.05,
            "min_samples": 50
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);

    // and exploration is capped well below "route at random"
    let too_much_exploration = client
        .put(format!("{base}/api/v1/adaptive-routing-policy"))
        .bearer_auth("sekrit")
        .json(&json!({
            "enabled": true,
            "latency_weight": 1.0,
            "cost_weight": 0.5,
            "load_weight": 0.25,
            "exploration_ratio": 0.9,
            "min_samples": 50
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_much_exploration.status(), 400);

    let updated: Value = client
        .put(format!("{base}/api/v1/adaptive-routing-policy"))
        .bearer_auth("sekrit")
        .json(&json!({
            "enabled": true,
            "latency_weight": 2.0,
            "cost_weight": 0.0,
            "load_weight": 0.5,
            "exploration_ratio": 0.1,
            "min_samples": 10
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["enabled"], true);
    assert_eq!(updated["latency_weight"], 2.0);
    assert_eq!(updated["min_samples"], 10);

    // the gateway snapshot carries the new policy without a restart
    let snap: Value = client
        .get(format!("{base}/internal/snapshot"))
        .bearer_auth("sekrit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snap["config"]["adaptive_routing"]["enabled"], true);
    assert_eq!(snap["config"]["adaptive_routing"]["latency_weight"], 2.0);
    assert_eq!(snap["config"]["adaptive_routing"]["min_samples"], 10);

    let action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'adaptive_routing_policy.update' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(action.as_deref(), Some("adaptive_routing_policy.update"));
}

/// End-to-end local-account login (ROL-32): seed a user with an argon2id hash
/// directly (no signup flow exists yet), then exercise login → `/auth/me` →
/// logout → the now-revoked token is rejected.
#[tokio::test]
async fn login_me_logout_round_trip() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app(pool.clone()).await.unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    // seed a user the way `rolter-seed` does (same argon2id hashing call shape)
    use argon2::password_hash::rand_core::OsRng;
    use argon2::password_hash::{PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2::Argon2::default()
        .hash_password(b"correct horse battery staple", &salt)
        .unwrap()
        .to_string();
    sqlx::query("insert into users (email, password_hash, is_superadmin) values ($1, $2, true)")
        .bind("admin@example.com")
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();

    // wrong password is rejected
    let denied = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"email": "admin@example.com", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);

    // /auth/me without a token is rejected
    let no_token = client
        .get(format!("{base}/api/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status(), 401);

    // a made-up token is rejected too (never matches a stored session hash)
    let bad_token = client
        .get(format!("{base}/api/v1/auth/me"))
        .bearer_auth("rolter_sess_not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_token.status(), 401);

    // correct credentials issue a session token
    let login: Value = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"email": "admin@example.com", "password": "correct horse battery staple"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["token"].as_str().expect("token").to_string();
    assert_eq!(login["user"]["email"], "admin@example.com");

    // the token resolves the current user via the CurrentUser extractor
    let me: Value = client
        .get(format!("{base}/api/v1/auth/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["user"]["email"], "admin@example.com");
    assert!(me["memberships"].is_array());

    // logout revokes the session; a repeat logout is a no-op (idempotent)
    let logout = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);
    let logout_again = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(logout_again.status(), 204);

    // the revoked token no longer resolves a session
    let after_logout = client
        .get(format!("{base}/api/v1/auth/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(after_logout.status(), 401);
}

/// An expired session row is rejected even though the token digest matches,
/// proving `find_active_by_hash`'s `expires_at > now()` bound is doing its job.
#[tokio::test]
async fn expired_session_is_rejected() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app(pool.clone()).await.unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let user_id: uuid::Uuid = sqlx::query_scalar(
        "insert into users (email, password_hash, is_superadmin) values ($1, $2, true) returning id",
    )
    .bind("expired@example.com")
    .bind("unused-hash")
    .fetch_one(&pool)
    .await
    .unwrap();

    // insert an already-expired session directly, bypassing login, with a
    // digest matching what the extractor computes for an empty pepper
    let token = "rolter_sess_deadbeef";
    let token_hash = rolter_auth::hash_key("", token);
    sqlx::query(
        "insert into sessions (user_id, token_hash, expires_at) values ($1, $2, now() - interval '1 hour')",
    )
    .bind(user_id)
    .bind(&token_hash)
    .execute(&pool)
    .await
    .unwrap();

    let resp = client
        .get(format!("{base}/api/v1/auth/me"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

/// Seed a local user directly (no signup flow) and return its id.
async fn seed_user(pool: &sqlx::PgPool, email: &str, is_superadmin: bool) -> uuid::Uuid {
    sqlx::query_scalar(
        "insert into users (email, password_hash, is_superadmin) values ($1, null, $2) returning id",
    )
    .bind(email)
    .bind(is_superadmin)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Grant `user` a role membership at an org/team/project scope (pass the ids
/// that apply; `None` for the levels that don't).
async fn seed_membership(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    org: Option<uuid::Uuid>,
    team: Option<uuid::Uuid>,
    project: Option<uuid::Uuid>,
    role: &str,
) {
    sqlx::query(
        "insert into memberships (user_id, org_id, team_id, project_id, role)
         values ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(org)
    .bind(team)
    .bind(project)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

/// Mint a live session for `user` and return the opaque bearer token. The
/// digest is computed with the empty pepper the extractor uses when
/// `ROLTER_SESSION_PEPPER` is unset in tests.
async fn seed_session(pool: &sqlx::PgPool, user_id: uuid::Uuid, suffix: &str) -> String {
    let token = format!("rolter_sess_{suffix}");
    let token_hash = rolter_auth::hash_key("", &token);
    sqlx::query(
        "insert into sessions (user_id, token_hash, expires_at)
         values ($1, $2, now() + interval '1 hour')",
    )
    .bind(user_id)
    .bind(&token_hash)
    .execute(pool)
    .await
    .unwrap();
    token
}

#[tokio::test]
async fn skill_access_policy_filters_list_history_and_resolution() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("admintok".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post_as(client: &reqwest::Client, url: String, body: Value) -> Value {
        let response = client
            .post(url)
            .bearer_auth("admintok")
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let json: Value = response.json().await.unwrap();
        assert!(status.is_success(), "{status}: {json}");
        json
    }

    let org = post_as(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().unwrap();
    let org_uuid: uuid::Uuid = org_id.parse().unwrap();
    let team_a = post_as(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Allowed"}),
    )
    .await;
    let team_b = post_as(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Denied"}),
    )
    .await;
    let team_a_uuid: uuid::Uuid = team_a["id"].as_str().unwrap().parse().unwrap();
    let team_b_uuid: uuid::Uuid = team_b["id"].as_str().unwrap().parse().unwrap();
    let skill = post_as(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/skills"),
        json!({
            "name": "classification",
            "allowed_team_ids": [team_a_uuid],
            "minimum_role": "member"
        }),
    )
    .await;
    let skill_id = skill["id"].as_str().unwrap();
    post_as(
        &client,
        format!("{base}/api/v1/skills/{skill_id}/versions"),
        json!({"content": "approved content"}),
    )
    .await;
    let publish = client
        .put(format!("{base}/api/v1/skills/{skill_id}/publish"))
        .bearer_auth("admintok")
        .json(&json!({"version": 1}))
        .send()
        .await
        .unwrap();
    assert!(publish.status().is_success());

    let allowed_user = seed_user(&pool, "allowed@example.com", false).await;
    seed_membership(&pool, allowed_user, Some(org_uuid), None, None, "member").await;
    seed_membership(
        &pool,
        allowed_user,
        Some(org_uuid),
        Some(team_a_uuid),
        None,
        "viewer",
    )
    .await;
    let allowed_token = seed_session(&pool, allowed_user, "skill-allowed").await;

    let denied_user = seed_user(&pool, "denied@example.com", false).await;
    seed_membership(&pool, denied_user, Some(org_uuid), None, None, "member").await;
    seed_membership(
        &pool,
        denied_user,
        Some(org_uuid),
        Some(team_b_uuid),
        None,
        "viewer",
    )
    .await;
    let denied_token = seed_session(&pool, denied_user, "skill-denied").await;

    let allowed_list: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/skills"))
        .bearer_auth(&allowed_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(allowed_list.as_array().unwrap().len(), 1);
    let denied_list: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/skills"))
        .bearer_auth(&denied_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(denied_list.as_array().unwrap().is_empty());

    let allowed_resolution = client
        .get(format!(
            "{base}/api/v1/orgs/{org_id}/skills/resolve/classification"
        ))
        .bearer_auth(&allowed_token)
        .send()
        .await
        .unwrap();
    assert!(allowed_resolution.status().is_success());
    let denied_history = client
        .get(format!("{base}/api/v1/skills/{skill_id}/versions"))
        .bearer_auth(&denied_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied_history.status(), 403);
    let denied_resolution = client
        .get(format!(
            "{base}/api/v1/orgs/{org_id}/skills/resolve/classification"
        ))
        .bearer_auth(&denied_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied_resolution.status(), 403);
}

/// MCP OAuth grants and sessions (#541): admins see the whole org, a member
/// sees only what they own, revoking a grant kills its sessions, and no token
/// material ever appears in a response.
#[tokio::test]
async fn mcp_oauth_grants_and_sessions_are_owner_scoped_and_revocable() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("admintok".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let org: Value = client
        .post(format!("{base}/api/v1/orgs"))
        .bearer_auth("admintok")
        .json(&json!({"name": "McpOrg", "slug": "mcp-org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let org_id = org["id"].as_str().unwrap().to_string();
    let org_uuid: uuid::Uuid = org_id.parse().unwrap();

    // a bad transport or a non-http url is refused before anything is stored
    for bad in [
        json!({"name": "S", "slug": "s", "url": "https://mcp.example.com", "transport": "carrier-pigeon"}),
        json!({"name": "S", "slug": "s", "url": "file:///etc/passwd"}),
    ] {
        let resp = client
            .post(format!("{base}/api/v1/orgs/{org_id}/mcp-servers"))
            .bearer_auth("admintok")
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "accepted {bad}");
    }

    let server: Value = client
        .post(format!("{base}/api/v1/orgs/{org_id}/mcp-servers"))
        .bearer_auth("admintok")
        .json(&json!({"name": "Docs", "slug": "docs", "url": "https://mcp.example.com"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(server["transport"], "streamable_http");
    let server_uuid: uuid::Uuid = server["id"].as_str().unwrap().parse().unwrap();

    // two members of the same org, each with their own grant and session
    let alice = seed_user(&pool, "alice@example.com", false).await;
    seed_membership(&pool, alice, Some(org_uuid), None, None, "member").await;
    let alice_token = seed_session(&pool, alice, "alice").await;
    let bob = seed_user(&pool, "bob@example.com", false).await;
    seed_membership(&pool, bob, Some(org_uuid), None, None, "member").await;
    let bob_token = seed_session(&pool, bob, "bob").await;

    let kek = rolter_store::postgres::crypto::Kek::from_secret("test-kek");
    let repo = rolter_store::postgres::repo::McpOAuthRepo(&pool);
    let alice_grant = repo
        .upsert_grant(server_uuid, alice, &["tools:read".to_string()])
        .await
        .unwrap();
    let bob_grant = repo
        .upsert_grant(server_uuid, bob, &["tools:read".to_string()])
        .await
        .unwrap();
    let alice_session = repo
        .store_session(
            &kek,
            alice_grant.id,
            "at-alice-secret",
            Some("rt-alice-secret"),
            &["tools:read".to_string()],
            chrono::Utc::now() + chrono::Duration::hours(1),
            Some(chrono::Utc::now() + chrono::Duration::days(30)),
        )
        .await
        .unwrap();
    repo.store_session(
        &kek,
        bob_grant.id,
        "at-bob-secret",
        None,
        &["tools:read".to_string()],
        chrono::Utc::now() + chrono::Duration::hours(1),
        None,
    )
    .await
    .unwrap();

    // the admin token sees both owners
    let all: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/mcp/grants"))
        .bearer_auth("admintok")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);

    // a member sees only their own grant and session
    let mine: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/mcp/grants"))
        .bearer_auth(&alice_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mine = mine.as_array().unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["user_id"], alice.to_string());
    assert_eq!(mine[0]["active"], true);

    let sessions: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/mcp/sessions"))
        .bearer_auth(&alice_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sessions_text = sessions.to_string();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["has_refresh_token"], true);
    // the response describes the session without ever carrying its tokens
    assert!(
        !sessions_text.contains("at-alice-secret") && !sessions_text.contains("rt-alice-secret"),
        "token material leaked into the sessions response: {sessions_text}"
    );

    // one member may not revoke another's session
    let denied = client
        .delete(format!("{base}/api/v1/mcp/sessions/{}", alice_session.id))
        .bearer_auth(&bob_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);

    // a member of a different org cannot even see it
    let other_org: Value = client
        .post(format!("{base}/api/v1/orgs"))
        .bearer_auth("admintok")
        .json(&json!({"name": "OtherMcpOrg", "slug": "other-mcp-org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let other_uuid: uuid::Uuid = other_org["id"].as_str().unwrap().parse().unwrap();
    let outsider = seed_user(&pool, "outsider@example.com", false).await;
    seed_membership(&pool, outsider, Some(other_uuid), None, None, "admin").await;
    let outsider_token = seed_session(&pool, outsider, "outsider").await;
    let cross_tenant = client
        .get(format!("{base}/api/v1/orgs/{org_id}/mcp/grants"))
        .bearer_auth(&outsider_token)
        .send()
        .await
        .unwrap();
    assert_eq!(cross_tenant.status(), 403);
    let cross_revoke = client
        .delete(format!("{base}/api/v1/mcp/grants/{}", alice_grant.id))
        .bearer_auth(&outsider_token)
        .send()
        .await
        .unwrap();
    assert_eq!(cross_revoke.status(), 403);

    // the sealed tokens open while the grant is live
    let opened = repo
        .open_session(&kek, alice_session.id, chrono::Utc::now())
        .await
        .unwrap()
        .expect("live session opens");
    assert_eq!(opened.access_token, "at-alice-secret");
    assert_eq!(opened.refresh_token.as_deref(), Some("rt-alice-secret"));

    // revoking the grant revokes its sessions in the same breath
    let revoked: Value = client
        .delete(format!("{base}/api/v1/mcp/grants/{}", alice_grant.id))
        .bearer_auth(&alice_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(revoked["active"], false);
    let after: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/mcp/sessions"))
        .bearer_auth(&alice_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(after[0]["revoked_at"].is_string());
    // and the tokens stop opening, so withdrawn consent cannot be spent
    assert!(repo
        .open_session(&kek, alice_session.id, chrono::Utc::now())
        .await
        .unwrap()
        .is_none());

    // an expired session is dead even though nobody revoked it
    let expired = repo
        .store_session(
            &kek,
            bob_grant.id,
            "at-bob-expired",
            None,
            &["tools:read".to_string()],
            chrono::Utc::now() - chrono::Duration::minutes(1),
            None,
        )
        .await
        .unwrap();
    assert!(repo
        .open_session(&kek, expired.id, chrono::Utc::now())
        .await
        .unwrap()
        .is_none());

    // and a KEK that did not seal the token cannot open it
    let wrong = rolter_store::postgres::crypto::Kek::from_secret("not-the-kek");
    let bob_live = repo
        .list_sessions(org_uuid, Some(bob))
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id != expired.id)
        .unwrap();
    assert!(repo
        .open_session(&wrong, bob_live.id, chrono::Utc::now())
        .await
        .is_err());

    let action: Option<String> = sqlx::query_scalar(
        "select action from audit_log where action = 'mcp_oauth_grant.revoke' order by at desc limit 1",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(action.as_deref(), Some("mcp_oauth_grant.revoke"));
}

/// The capability matrix and the caller's effective permissions are served by
/// the control plane rather than assembled in the browser (#534): the matrix
/// describes the rules, `effective` answers for the caller at a scope, and the
/// answer matches what the CRUD guard actually does.
#[tokio::test]
async fn rbac_matrix_and_effective_permissions_are_api_backed() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("admintok".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    // the matrix needs authentication but no particular role
    let unauth = client
        .get(format!("{base}/api/v1/rbac/matrix"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    let matrix: Value = client
        .get(format!("{base}/api/v1/rbac/matrix"))
        .bearer_auth("admintok")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(matrix["roles"].as_array().unwrap().len(), 3);
    let provider = matrix["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["resource"] == "provider")
        .expect("provider in the matrix");
    let create = provider["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["action"] == "create")
        .unwrap();
    assert_eq!(create["minimum_role"], "admin");
    assert_eq!(create["superadmin_only"], false);
    // deployment-wide policy is not something an org admin can reach
    let flags = matrix["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["resource"] == "feature_flags")
        .expect("feature_flags in the matrix");
    assert_eq!(flags["actions"][0]["superadmin_only"], true);

    let org: Value = client
        .post(format!("{base}/api/v1/orgs"))
        .bearer_auth("admintok")
        .json(&json!({"name": "MatrixOrg", "slug": "matrix-org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let org_id = org["id"].as_str().unwrap().to_string();
    let org_uuid: uuid::Uuid = org_id.parse().unwrap();

    let viewer = seed_user(&pool, "matrix-viewer@example.com", false).await;
    seed_membership(&pool, viewer, Some(org_uuid), None, None, "viewer").await;
    let viewer_token = seed_session(&pool, viewer, "matrixviewer").await;

    let effective: Value = client
        .get(format!("{base}/api/v1/rbac/effective?org_id={org_id}"))
        .bearer_auth(&viewer_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(effective["superadmin"], false);
    assert_eq!(effective["role"], "viewer");
    let allowed: Vec<String> = serde_json::from_value(effective["allowed"].clone()).unwrap();
    assert!(allowed.contains(&"provider:read".to_string()));
    assert!(!allowed.contains(&"provider:create".to_string()));

    // and the guard agrees with what `effective` reported
    let denied = client
        .post(format!("{base}/api/v1/orgs/{org_id}/teams"))
        .bearer_auth(&viewer_token)
        .json(&json!({"name": "T"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);

    // an org the caller has no membership in yields no permissions at all
    let other: Value = client
        .post(format!("{base}/api/v1/orgs"))
        .bearer_auth("admintok")
        .json(&json!({"name": "OtherOrg", "slug": "other-org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let other_id = other["id"].as_str().unwrap();
    let elsewhere: Value = client
        .get(format!("{base}/api/v1/rbac/effective?org_id={other_id}"))
        .bearer_auth(&viewer_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(elsewhere["role"].is_null());
    assert_eq!(elsewhere["allowed"].as_array().unwrap().len(), 0);

    // a project-scoped admin inherits nothing upward: the same user is only an
    // admin inside the project chain they were granted
    let team: Value = client
        .post(format!("{base}/api/v1/orgs/{org_id}/teams"))
        .bearer_auth("admintok")
        .json(&json!({"name": "MatrixTeam"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let team_id = team["id"].as_str().unwrap().to_string();
    let team_uuid: uuid::Uuid = team_id.parse().unwrap();
    let project: Value = client
        .post(format!("{base}/api/v1/teams/{team_id}/projects"))
        .bearer_auth("admintok")
        .json(&json!({"name": "MatrixProject"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let project_id = project["id"].as_str().unwrap().to_string();
    let project_uuid: uuid::Uuid = project_id.parse().unwrap();

    let scoped = seed_user(&pool, "matrix-project-admin@example.com", false).await;
    seed_membership(
        &pool,
        scoped,
        Some(org_uuid),
        Some(team_uuid),
        Some(project_uuid),
        "admin",
    )
    .await;
    let scoped_token = seed_session(&pool, scoped, "matrixscoped").await;

    let in_project: Value = client
        .get(format!(
            "{base}/api/v1/rbac/effective?org_id={org_id}&team_id={team_id}&project_id={project_id}"
        ))
        .bearer_auth(&scoped_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(in_project["role"], "admin");

    let at_org: Value = client
        .get(format!("{base}/api/v1/rbac/effective?org_id={org_id}"))
        .bearer_auth(&scoped_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        at_org["role"].is_null(),
        "a project-scoped grant must not authorize the whole org"
    );
}

/// With an admin token configured (RBAC enforcement active), every control
/// mutation is checked against the caller's role at the resource's scope:
/// viewers are denied, scoped admins are allowed only within their scope,
/// cross-scope admins are denied, superadmins bypass, and the machine admin
/// token bypasses. Covers ROL-33 (resolver) + ROL-34 (enforcement).
#[tokio::test]
async fn rbac_enforced_on_every_mutation() {
    skip_without_db!();
    let pool = fresh_pool().await;
    let app = rolter_control::test_app_with_admin_token(pool.clone(), Some("admintok".to_string()))
        .await
        .unwrap();
    let addr = serve(app).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    // bootstrap the org hierarchy as the machine admin token (superadmin)
    async fn post_as(client: &reqwest::Client, url: &str, token: &str, body: Value) -> Value {
        let resp = client
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    let org_a = post_as(
        &client,
        &format!("{base}/api/v1/orgs"),
        "admintok",
        json!({"name": "OrgA", "slug": "org-a"}),
    )
    .await;
    let org_a_id = org_a["id"].as_str().unwrap().to_string();
    let org_b = post_as(
        &client,
        &format!("{base}/api/v1/orgs"),
        "admintok",
        json!({"name": "OrgB", "slug": "org-b"}),
    )
    .await;
    let org_b_id = org_b["id"].as_str().unwrap().to_string();

    let org_a_uuid: uuid::Uuid = org_a_id.parse().unwrap();
    let org_b_uuid: uuid::Uuid = org_b_id.parse().unwrap();

    // seed principals: a viewer and an admin on org A, an admin on org B, and a
    // superadmin with no memberships at all
    let viewer = seed_user(&pool, "viewer@example.com", false).await;
    seed_membership(&pool, viewer, Some(org_a_uuid), None, None, "viewer").await;
    let viewer_token = seed_session(&pool, viewer, "viewer").await;

    let admin_a = seed_user(&pool, "admin-a@example.com", false).await;
    seed_membership(&pool, admin_a, Some(org_a_uuid), None, None, "admin").await;
    let admin_a_token = seed_session(&pool, admin_a, "admina").await;

    let admin_b = seed_user(&pool, "admin-b@example.com", false).await;
    seed_membership(&pool, admin_b, Some(org_b_uuid), None, None, "admin").await;
    let admin_b_token = seed_session(&pool, admin_b, "adminb").await;

    let super_user = seed_user(&pool, "super@example.com", true).await;
    let super_token = seed_session(&pool, super_user, "super").await;

    let create_team_url = format!("{base}/api/v1/orgs/{org_a_id}/teams");

    // unauthenticated → 401 (RBAC enforcement is active)
    let unauth = client
        .post(&create_team_url)
        .json(&json!({"name": "T"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401, "no credentials must be rejected");

    // viewer on org A → 403 creating a team (mutation needs admin)
    let viewer_denied = client
        .post(&create_team_url)
        .bearer_auth(&viewer_token)
        .json(&json!({"name": "T-viewer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(viewer_denied.status(), 403, "viewer must not create");

    // admin on org A → allowed on org A
    let admin_ok = client
        .post(&create_team_url)
        .bearer_auth(&admin_a_token)
        .json(&json!({"name": "T-admin"}))
        .send()
        .await
        .unwrap();
    assert!(
        admin_ok.status().is_success(),
        "org-A admin must create under org A: {}",
        admin_ok.status()
    );

    // admin on org B → 403 creating a provider under org A (cross-scope)
    let cross_scope = client
        .post(format!("{base}/api/v1/orgs/{org_a_id}/providers"))
        .bearer_auth(&admin_b_token)
        .json(&json!({"name": "p1", "kind": "openai", "api_base": "https://api.openai.com"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        cross_scope.status(),
        403,
        "org-B admin must not mutate org-A resources"
    );

    // superadmin session → bypass, allowed even with no membership
    let super_ok = client
        .post(&create_team_url)
        .bearer_auth(&super_token)
        .json(&json!({"name": "T-super"}))
        .send()
        .await
        .unwrap();
    assert!(
        super_ok.status().is_success(),
        "superadmin must bypass: {}",
        super_ok.status()
    );

    // global model-price catalog is superadmin-only: the org-A admin is denied
    let price_denied = client
        .put(format!("{base}/api/v1/model-prices"))
        .bearer_auth(&admin_a_token)
        .json(&json!({"model": "gpt-4o", "input_per_mtok": "1", "output_per_mtok": "2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        price_denied.status(),
        403,
        "model-price mutation is superadmin-only"
    );
    let price_ok = client
        .put(format!("{base}/api/v1/model-prices"))
        .bearer_auth("admintok")
        .json(&json!({"model": "gpt-4o", "input_per_mtok": "1", "output_per_mtok": "2"}))
        .send()
        .await
        .unwrap();
    assert!(price_ok.status().is_success(), "{}", price_ok.status());
}

/// Open mode (no admin token) must keep the CRUD API fully open: an
/// unauthenticated mutation still succeeds, preserving zero-cred local dev.
#[tokio::test]
async fn open_mode_allows_unauthenticated_mutations() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/v1/orgs"))
        .json(&json!({"name": "Acme", "slug": "acme"}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "open mode must allow unauthenticated mutations: {}",
        resp.status()
    );
}

/// Full user lifecycle (ROL-223): invite an account into an org, list it, grant
/// a team-scoped role, then deactivate it and confirm login is blocked while the
/// row and its memberships survive.
#[tokio::test]
async fn user_and_membership_lifecycle() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    // org → team scaffold
    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().unwrap().to_string();
    let team = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Platform"}),
    )
    .await;
    let team_id = team["id"].as_str().unwrap().to_string();

    // invite a user into the org with an initial password + role
    let created = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/users"),
        json!({"email": "dev@example.com", "password": "hunter2!!", "role": "member"}),
    )
    .await;
    let user_id = created["user"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["user"]["email"], "dev@example.com");
    assert_eq!(created["user"]["is_superadmin"], false);
    // the password hash must never be serialized back
    assert!(created["user"].get("password_hash").is_none());
    assert_eq!(created["membership"]["role"], "member");

    // the account shows up in the org's user list
    let users: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/users"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        users.as_array().unwrap().iter().any(|u| u["id"] == user_id),
        "invited user missing from org list: {users}"
    );

    // duplicate email is a conflict
    let dup = client
        .post(format!("{base}/api/v1/orgs/{org_id}/users"))
        .json(&json!({"email": "dev@example.com", "password": "hunter2!!"}))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), 409);

    // grant a team-scoped admin role
    let membership = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/memberships"),
        json!({"user_id": user_id, "scope_type": "team", "scope_id": team_id, "role": "admin"}),
    )
    .await;
    let membership_id = membership["id"].as_str().unwrap().to_string();
    assert_eq!(membership["team_id"], team_id);

    // both memberships (org member + team admin) are listed for the org
    let memberships: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/memberships"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        memberships.as_array().unwrap().len(),
        2,
        "expected org + team memberships: {memberships}"
    );

    // the account can log in before deactivation
    let ok = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"email": "dev@example.com", "password": "hunter2!!"}))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200, "login should succeed before deactivation");

    // deactivate the account
    let deact = client
        .put(format!("{base}/api/v1/users/{user_id}"))
        .json(&json!({"deactivated": true}))
        .send()
        .await
        .unwrap();
    assert!(deact.status().is_success());
    let deact_body: Value = deact.json().await.unwrap();
    assert!(deact_body["deactivated_at"].is_string());

    // login is now blocked, but the user + memberships still exist
    let blocked = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"email": "dev@example.com", "password": "hunter2!!"}))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 401, "deactivated account must not log in");
    let still: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/memberships"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(still.as_array().unwrap().len(), 2);

    // revoke the team membership, then delete the account
    let del_m = client
        .delete(format!("{base}/api/v1/memberships/{membership_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del_m.status(), 204);
    let del_u = client
        .delete(format!("{base}/api/v1/users/{user_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del_u.status(), 204);

    // the org user list is empty again (cascade removed the org membership too)
    let after: Value = client
        .get(format!("{base}/api/v1/orgs/{org_id}/users"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        after.as_array().unwrap().is_empty(),
        "user should be gone after delete: {after}"
    );
}

/// Self-service key lifecycle (ROL-224): a logged-in member mints, lists,
/// rotates and deletes their own virtual keys, and usage 503s without
/// ClickHouse. Runs in open mode; `/me/*` still requires a real session.
#[tokio::test]
async fn self_service_key_lifecycle() {
    skip_without_db!();
    let addr = serve(fresh_app().await).await;
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    async fn post(client: &reqwest::Client, url: String, body: Value) -> Value {
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let status = resp.status();
        let json: Value = resp.json().await.unwrap();
        assert!(status.is_success(), "POST {url} failed ({status}): {json}");
        json
    }

    // org → team → project
    let org = post(
        &client,
        format!("{base}/api/v1/orgs"),
        json!({"name": "Acme", "slug": "acme"}),
    )
    .await;
    let org_id = org["id"].as_str().unwrap().to_string();
    let team = post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/teams"),
        json!({"name": "Platform"}),
    )
    .await;
    let team_id = team["id"].as_str().unwrap().to_string();
    let project = post(
        &client,
        format!("{base}/api/v1/teams/{team_id}/projects"),
        json!({"name": "Gateway"}),
    )
    .await;
    let project_id = project["id"].as_str().unwrap().to_string();

    // invite a member into the org (org membership authorizes the project too)
    post(
        &client,
        format!("{base}/api/v1/orgs/{org_id}/users"),
        json!({"email": "member@example.com", "password": "hunter2!!", "role": "member"}),
    )
    .await;

    // log in to get a session token
    let login = post(
        &client,
        format!("{base}/api/v1/auth/login"),
        json!({"email": "member@example.com", "password": "hunter2!!"}),
    )
    .await;
    let token = login["token"].as_str().unwrap().to_string();

    // /me/* requires a session: unauthenticated is rejected
    let anon = client
        .get(format!("{base}/api/v1/me/virtual-keys"))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);

    // mint a key I own in the project I belong to
    let minted = client
        .post(format!(
            "{base}/api/v1/me/projects/{project_id}/virtual-keys"
        ))
        .bearer_auth(&token)
        .json(&json!({"name": "laptop", "models": ["gpt-4o"]}))
        .send()
        .await
        .unwrap();
    assert!(minted.status().is_success());
    let minted: Value = minted.json().await.unwrap();
    assert!(minted["key"].as_str().unwrap().starts_with("sk-rolter-"));
    let key_id = minted["id"].as_str().unwrap().to_string();

    // it shows up in my key list, enriched with project/org names
    let keys: Value = client
        .get(format!("{base}/api/v1/me/virtual-keys"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = keys.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["project_name"], "Gateway");
    assert_eq!(arr[0]["org_name"], "Acme");
    // the key hash is never exposed on the self-service surface
    assert!(arr[0].get("key_hash").is_none());

    // rotate: a new secret, old key disabled, both still owned/listed
    let rotated = client
        .post(format!("{base}/api/v1/me/virtual-keys/{key_id}/rotate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(rotated.status().is_success());
    let rotated: Value = rotated.json().await.unwrap();
    let new_id = rotated["id"].as_str().unwrap().to_string();
    assert_ne!(new_id, key_id);

    let after: Value = client
        .get(format!("{base}/api/v1/me/virtual-keys"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let after = after.as_array().unwrap();
    assert_eq!(after.len(), 2);
    let old = after.iter().find(|k| k["id"] == key_id).unwrap();
    assert_eq!(old["disabled"], true, "rotated-out key must be disabled");

    // usage 503s without ClickHouse configured
    let usage = client
        .get(format!("{base}/api/v1/me/usage"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(usage.status(), 503);

    // delete the new key
    let del = client
        .delete(format!("{base}/api/v1/me/virtual-keys/{new_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);
    let remaining: Value = client
        .get(format!("{base}/api/v1/me/virtual-keys"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(remaining.as_array().unwrap().len(), 1);
}
