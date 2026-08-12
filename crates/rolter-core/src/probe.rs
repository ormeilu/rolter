//! Where to reach a provider to find out whether it is alive.
//!
//! Both planes need this and they must agree. The gateway sweeps providers for
//! the health screen; the control plane answers "Test connection" when an
//! operator is configuring one. If the two built the URL separately they would
//! drift the moment a provider kind was added, and an operator would get a green
//! test against an endpoint the health sweep never probes — the same failure
//! mode as the balancing-strategy mapping in #930, which fell behind twice.
//!
//! Every endpoint here is a **free, non-inference** call. A liveness check that
//! bills the operator is one they will turn off.

use crate::ProviderKind;

/// The anthropic messages api rejects requests without a version header, even on
/// the free `GET /v1/models` list endpoint.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Resolve the probe request for a provider: the URL and any header the upstream
/// requires.
///
/// When the operator left `health.path` at its default (`/`), probe the provider
/// kind's free liveness endpoint (`/v1/models` — a list call that burns no
/// tokens) so a healthy result means the API itself is up, not merely that the
/// host answers TCP. An explicit non-default `path` is honoured verbatim for
/// every provider.
pub fn probe_request(
    kind: ProviderKind,
    api_base: &str,
    configured_path: &str,
) -> (String, Vec<(String, String)>) {
    let base = api_base.trim_end_matches('/');
    if configured_path != "/" {
        return (format!("{base}{configured_path}"), Vec::new());
    }
    match kind {
        ProviderKind::Anthropic => (
            format!("{base}/v1/models"),
            vec![(
                "anthropic-version".to_string(),
                ANTHROPIC_VERSION.to_string(),
            )],
        ),
        ProviderKind::Tei => (format!("{base}/health"), Vec::new()),
        ProviderKind::Bedrock => (bedrock_models_url(base), Vec::new()),
        ProviderKind::Vertex => (vertex_models_url(base), Vec::new()),
        ProviderKind::Openai
        | ProviderKind::OpenaiCompatible
        | ProviderKind::Ollama
        | ProviderKind::OllamaCloud
        | ProviderKind::LlamaCpp => (format!("{base}/v1/models"), Vec::new()),
        _ => (format!("{base}/models"), Vec::new()),
    }
}

fn bedrock_models_url(api_base: &str) -> String {
    let base = api_base.trim_end_matches('/');
    if let Some(control) = base.strip_prefix("https://bedrock-runtime.") {
        let host = control.split('/').next().unwrap_or(control);
        return format!("https://bedrock.{host}/foundation-models");
    }
    format!("{}/foundation-models", base.trim_end_matches("/v1"))
}

fn vertex_models_url(api_base: &str) -> String {
    let base = api_base.trim_end_matches('/');
    if let Some(prefix) = base.strip_suffix("/endpoints/openapi") {
        return format!("{prefix}/publishers/google/models");
    }
    format!("{base}/models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_shaped_kinds_probe_the_free_model_list() {
        let (url, hdr) = probe_request(ProviderKind::Openai, "https://api.openai.com/", "/");
        assert_eq!(url, "https://api.openai.com/v1/models");
        assert!(hdr.is_empty());

        let (url, _) = probe_request(ProviderKind::OpenaiCompatible, "http://vllm:8000", "/");
        assert_eq!(url, "http://vllm:8000/v1/models");
    }

    #[test]
    fn anthropic_carries_the_version_header_it_requires() {
        let (url, hdr) = probe_request(ProviderKind::Anthropic, "https://api.anthropic.com", "/");
        assert_eq!(url, "https://api.anthropic.com/v1/models");
        assert_eq!(
            hdr,
            vec![(
                "anthropic-version".to_string(),
                ANTHROPIC_VERSION.to_string()
            )]
        );
    }

    // tei serves embeddings, not a model catalogue, so /v1/models would 404 and
    // report a live provider as down
    #[test]
    fn tei_probes_its_health_endpoint() {
        let (url, headers) = probe_request(ProviderKind::Tei, "http://tei:80/", "/");
        assert_eq!(url, "http://tei:80/health");
        assert!(headers.is_empty());
    }

    #[test]
    fn bedrock_and_vertex_rewrite_onto_their_control_planes() {
        let (url, _) = probe_request(
            ProviderKind::Bedrock,
            "https://bedrock-runtime.us-east-1.amazonaws.com/v1",
            "/",
        );
        assert_eq!(
            url,
            "https://bedrock.us-east-1.amazonaws.com/foundation-models"
        );

        let (url, _) = probe_request(
            ProviderKind::Vertex,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/endpoints/openapi",
            "/",
        );
        assert_eq!(
            url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/publishers/google/models"
        );
    }

    #[test]
    fn an_explicit_path_wins_over_the_kind_default() {
        let (url, hdr) = probe_request(ProviderKind::Anthropic, "https://x.test", "/healthz");
        assert_eq!(url, "https://x.test/healthz");
        assert!(hdr.is_empty(), "an explicit path carries no kind headers");
    }
}
