//! What the PII sanitizer costs *inside* the gateway, per request.
//!
//! The network round trip to the sanitizer service dominates wall-clock time
//! and is not rolter's to measure. What rolter owns is the work on either side
//! of that call: bounding the body, encoding the envelope, decoding the reply
//! and deciding whether the reply's token may be used in this tenant's scope.
//! That is what runs on the hot path for every sanitized request, so that is
//! what is measured here.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rolter_core::pii_sanitizer::bound_content;
use rolter_core::{
    Finding, RestorationTicket, SanitizeRequest, SanitizeResponse, TokenScope, WebhookTenant,
};
use serde_json::{json, Value};
use std::hint::black_box;

/// An OpenAI chat body with `turns` messages, each carrying an address the
/// sanitizer would replace.
fn chat_body(turns: usize) -> Value {
    let messages: Vec<Value> = (0..turns)
        .map(|i| {
            json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": format!(
                    "turn {i}: contact ops{i}@corp.example about invoice 4111-1111-1111-{i:04}"
                ),
            })
        })
        .collect();
    json!({"model": "gpt-4o-mini", "messages": messages})
}

fn tenant() -> WebhookTenant {
    WebhookTenant {
        org: Some("acme".to_string()),
        team: Some("platform".to_string()),
        project: Some("chat".to_string()),
        key: None,
    }
}

fn scope() -> TokenScope {
    TokenScope {
        org: "acme".to_string(),
        team: "platform".to_string(),
        project: "chat".to_string(),
        route: "gpt-4o-mini".to_string(),
    }
}

fn bench_envelope(c: &mut Criterion) {
    let mut group = c.benchmark_group("pii/envelope");
    let entities = vec!["EMAIL_ADDRESS".to_string(), "CREDIT_CARD".to_string()];
    for turns in [1_usize, 8, 64] {
        let body = chat_body(turns);
        group.bench_with_input(BenchmarkId::from_parameter(turns), &turns, |b, _| {
            b.iter(|| {
                let (payload, truncated) = bound_content(black_box(&body), 256 * 1024);
                let envelope = SanitizeRequest {
                    direction: "request",
                    model: "gpt-4o-mini",
                    route: "gpt-4o-mini",
                    trace_id: "0af7651916cd43dd8448eb211c80319c",
                    tenant: tenant(),
                    entities: &entities,
                    reversible: true,
                    truncated,
                    content: &payload,
                };
                black_box(serde_json::to_vec(&envelope).unwrap())
            })
        });
    }
    group.finish();
}

/// The truncation path specifically: a body over the ceiling costs a full
/// `to_string` plus a char-wise walk, which is the worst case worth knowing.
fn bench_bound_oversized(c: &mut Criterion) {
    let body = chat_body(4096);
    let mut group = c.benchmark_group("pii/bound");
    group.bench_function("under_ceiling", |b| {
        b.iter(|| black_box(bound_content(black_box(&body), 64 * 1024 * 1024)))
    });
    group.bench_function("truncated", |b| {
        b.iter(|| black_box(bound_content(black_box(&body), 16 * 1024)))
    });
    group.finish();
}

fn bench_reply(c: &mut Criterion) {
    let mut group = c.benchmark_group("pii/reply");
    for turns in [1_usize, 8, 64] {
        let reply = serde_json::to_vec(&SanitizeResponse {
            content: chat_body(turns),
            findings: vec![Finding {
                entity_type: "EMAIL_ADDRESS".to_string(),
                count: turns,
                placeholders: (0..turns).map(|i| format!("<EMAIL_ADDRESS_{i}>")).collect(),
            }],
            restoration_token: Some("tok-0123456789abcdef".to_string()),
        })
        .unwrap();
        group.bench_with_input(BenchmarkId::from_parameter(turns), &turns, |b, _| {
            b.iter(|| {
                let parsed: SanitizeResponse = serde_json::from_slice(black_box(&reply)).unwrap();
                black_box(parsed.replaced())
            })
        });
    }
    group.finish();
}

/// The scope check gates every restore, so it must not be the expensive part.
fn bench_scope_check(c: &mut Criterion) {
    let ticket = RestorationTicket::new("tok-0123456789abcdef".to_string(), scope());
    let same = scope();
    let mut other = scope();
    other.project = "billing".to_string();
    c.bench_function("pii/scope_check", |b| {
        b.iter(|| {
            black_box(ticket.token_for(black_box(&same)));
            black_box(ticket.token_for(black_box(&other)))
        })
    });
}

criterion_group!(
    benches,
    bench_envelope,
    bench_bound_oversized,
    bench_reply,
    bench_scope_check
);
criterion_main!(benches);
