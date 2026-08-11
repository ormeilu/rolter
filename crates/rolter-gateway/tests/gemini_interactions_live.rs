//! Opt-in credentialed smoke test for the `gemini_interactions` dialect (#764).
//!
//! #761 unit-tested the adapter against the interactions wire format *as
//! documented*, which is as far as static verification goes: Google does not
//! publish a full JSON schema for it, so some field names — the multimodal part
//! shape in particular — were inferred from the overview and migration pages.
//!
//! Confirming those against the real endpoint needs a live `GEMINI_API_KEY`,
//! which cannot run in the per-PR gate: `quality.yml` deliberately takes no
//! secrets (#734) so dependabot and fork PRs pass the same checks. These tests
//! are therefore `#[ignore]`d and run from the dispatch-gated
//! `gemini-interactions-smoke` workflow, matching how the OpenRouter and Ollama
//! Cloud live tests are gated.
//!
//! Run locally with:
//!
//! ```text
//! GEMINI_API_KEY=... ROLTER_GEMINI_LIVE_MODEL=gemini-3.6-flash \
//!   cargo test -p rolter-gateway --test gemini_interactions_live -- --ignored
//! ```

use std::net::SocketAddr;

use axum::http::StatusCode;
use axum::Router;
use rolter_core::{
    BalancingStrategy, GatewayConfig, ModelRoute, ProviderConfig, ProviderKind, Target,
};
use serde_json::{json, Value};

const DEFAULT_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

/// Credentials and model come from the environment so the suite never carries a
/// key and never pins a model that Google may retire.
fn live_env() -> (String, String) {
    let key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY");
    let model = std::env::var("ROLTER_GEMINI_LIVE_MODEL")
        .unwrap_or_else(|_| "gemini-3.6-flash".to_string());
    (key, model)
}

fn live_gateway(key: String, model: String) -> GatewayConfig {
    let provider = ProviderConfig {
        name: "gemini-interactions".into(),
        slug: None,
        kind: ProviderKind::GeminiInteractions,
        api_base: std::env::var("ROLTER_GEMINI_API_BASE")
            .unwrap_or_else(|_| DEFAULT_API_BASE.to_string()),
        api_key: Some(key),
        api_key_env: None,
        egress_proxy: None,
        egress_proxies: Vec::new(),
        kv_events: None,
        lmcache: None,
        ca_bundles: None,
        api_keys: vec![],
        also_track_via_llm_call: false,
        llm_probe_model: None,
        status_page_url: None,
        role_profile: None,
        model_role_profiles: Default::default(),
    };
    let mut config = GatewayConfig::default();
    config.routes.push(ModelRoute {
        model: "gemini-live".into(),
        strategy: BalancingStrategy::RoundRobin,
        targets: vec![Target {
            provider: provider.name.clone(),
            model: Some(model),
            weight: 1,
        }],
        params: Default::default(),
        param_policy: Default::default(),
        advanced: Default::default(),
        cache: None,
        variants: Default::default(),
    });
    config.providers = vec![provider];
    config
}

/// Fail with the upstream body rather than a bare status code. A malformed
/// field name comes back as a 400 whose message names the offending field —
/// that message is the entire point of running this test, so it must not be
/// swallowed by `assert_eq!(status, 200)`.
async fn expect_ok(response: reqwest::Response, what: &str) -> Value {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(
        status,
        StatusCode::OK,
        "{what} failed with {status}; upstream said: {body}"
    );
    serde_json::from_str(&body).unwrap_or(Value::Null)
}

/// The documented path: a text turn through the OpenAI dialect. Everything this
/// exercises (turn mapping, `system_instruction`, `generation_config`, usage)
/// follows shapes Google documents, so a failure here is a real regression
/// rather than a field-name discovery.
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY and makes a billable network request"]
async fn live_text_turn_round_trips() {
    let (key, model) = live_env();
    let gateway = serve(rolter_gateway::build_router_from_config(&live_gateway(
        key, model,
    )))
    .await;

    let response = reqwest::Client::new()
        .post(format!("http://{gateway}/v1/chat/completions"))
        .json(&json!({
            "model": "gemini-live",
            "messages": [
                {"role": "system", "content": "Reply with exactly one word."},
                {"role": "user", "content": "Say OK"}
            ],
            "max_tokens": 8
        }))
        .send()
        .await
        .unwrap();

    let body = expect_ok(response, "text turn").await;
    assert!(
        body["choices"][0]["message"]["content"].is_string(),
        "no assistant content in {body}"
    );
    // usage mapping is documented, so this asserts rather than merely reports
    assert!(
        body["usage"]["prompt_tokens"].is_number(),
        "no usage in {body}"
    );
}

/// Streaming: confirms the SSE converter emits well-formed chunks and
/// terminates. `step.delta` variants are among the shapes #764 flags as
/// inferred, so a body that arrives but yields no content is reported
/// explicitly rather than passing on the 200 alone.
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY and makes a billable network request"]
async fn live_stream_terminates_and_carries_content() {
    let (key, model) = live_env();
    let gateway = serve(rolter_gateway::build_router_from_config(&live_gateway(
        key, model,
    )))
    .await;

    let response = reqwest::Client::new()
        .post(format!("http://{gateway}/v1/chat/completions"))
        .json(&json!({
            "model": "gemini-live",
            "messages": [{"role": "user", "content": "Count to three."}],
            "max_tokens": 32,
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(status, StatusCode::OK, "stream failed: {body}");
    assert!(
        content_type.contains("text/event-stream"),
        "not an SSE response ({content_type}): {body}"
    );
    assert!(
        body.contains("data: [DONE]"),
        "stream never terminated: {body}"
    );
    // an SSE stream that terminates but carried no delta means the converter
    // did not recognize the event shape — the exact failure #764 is looking for
    assert!(
        body.contains("\"delta\""),
        "stream carried no delta chunks; the step.delta shape may differ: {body}"
    );
}

/// The multimodal path, which is the least certain part of the adapter: the
/// image part field names (`mime_type`/`data` for inline, `file_uri` for a
/// remote URL) were inferred, not read off a schema.
///
/// A 400 here is the finding, not a flake — the assertion message carries the
/// upstream complaint so the corrected field name can go straight into
/// `crates/rolter-proxy/src/translation.rs`.
#[tokio::test]
#[ignore = "requires GEMINI_API_KEY and makes a billable network request"]
async fn live_inline_image_part_is_accepted() {
    let (key, model) = live_env();
    let gateway = serve(rolter_gateway::build_router_from_config(&live_gateway(
        key, model,
    )))
    .await;

    // a 1x1 png, inline as a data URL — the adapter turns this into an inline
    // image part rather than a file reference
    const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    let response = reqwest::Client::new()
        .post(format!("http://{gateway}/v1/chat/completions"))
        .json(&json!({
            "model": "gemini-live",
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "What colour is this image? One word."},
                {"type": "image_url", "image_url": {"url": PIXEL}}
            ]}],
            "max_tokens": 16
        }))
        .send()
        .await
        .unwrap();

    let body = expect_ok(response, "inline image part").await;
    assert!(
        body["choices"][0]["message"]["content"].is_string(),
        "image turn returned no content in {body}"
    );
}
