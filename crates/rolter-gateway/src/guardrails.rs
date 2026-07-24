//! Request-path wiring for the built-in guardrails (ROL-261).
//!
//! Walks the text-bearing fields of an OpenAI- or Anthropic-shaped request body
//! and applies the compiled pre-call rules from the snapshot. Redactions are
//! written back into the JSON in place; a blocking match returns the offending
//! rule name so the handler can reject with an OpenAI-compatible error. The walk
//! only ever passes bounded text slices to the engine, and the returned report
//! carries rule names and counts only — never the matched values.

use rolter_core::guardrails::RuleSelection;
use rolter_core::{CompiledGuardrails, GuardrailReport, ScanOutcome};
use serde_json::Value;

/// Apply pre-call guardrails to a parsed request body for the given API path.
///
/// Returns `Err(rule_name)` when a rule blocks the request. On `Ok`, the body may
/// have been mutated in place (redactions); `report.redactions > 0` signals the
/// caller should re-serialize before forwarding.
pub fn apply_input(
    guardrails: &CompiledGuardrails,
    selection: &RuleSelection,
    path: &str,
    body: &mut Value,
) -> Result<GuardrailReport, String> {
    let mut report = GuardrailReport::default();
    let mut budget = guardrails.scan_budget();

    match path {
        // OpenAI chat + Responses share the `messages` shape; Responses also
        // accepts a bare `input` string/array
        "/v1/chat/completions" | "/v1/responses" => {
            scan_messages(guardrails, selection, body, &mut budget, &mut report)?;
            if let Some(input) = body.get_mut("input") {
                scan_content(
                    guardrails,
                    selection,
                    input,
                    false,
                    &mut budget,
                    &mut report,
                )?;
            }
        }
        // legacy text completions carry a `prompt` string or array of strings
        "/v1/completions" => {
            if let Some(prompt) = body.get_mut("prompt") {
                scan_content(
                    guardrails,
                    selection,
                    prompt,
                    false,
                    &mut budget,
                    &mut report,
                )?;
            }
        }
        // Anthropic Messages: a top-level `system` plus user/assistant `messages`
        "/v1/messages" => {
            if let Some(system) = body.get_mut("system") {
                scan_content(
                    guardrails,
                    selection,
                    system,
                    true,
                    &mut budget,
                    &mut report,
                )?;
            }
            scan_messages(guardrails, selection, body, &mut budget, &mut report)?;
        }
        _ => {}
    }

    Ok(report)
}

/// Scan every message in a `messages` array, treating `system`/`developer` roles
/// as operator-authored system content.
fn scan_messages(
    guardrails: &CompiledGuardrails,
    selection: &RuleSelection,
    body: &mut Value,
    budget: &mut usize,
    report: &mut GuardrailReport,
) -> Result<(), String> {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for message in messages {
        let is_system = matches!(
            message.get("role").and_then(Value::as_str),
            Some("system") | Some("developer")
        );
        if let Some(content) = message.get_mut("content") {
            scan_content(guardrails, selection, content, is_system, budget, report)?;
        }
    }
    Ok(())
}

/// Scan a `content`/`prompt`/`system` value that may be a string, an array of
/// strings, or an array of typed parts (`{ "type": "text", "text": "…" }`).
fn scan_content(
    guardrails: &CompiledGuardrails,
    selection: &RuleSelection,
    value: &mut Value,
    is_system: bool,
    budget: &mut usize,
    report: &mut GuardrailReport,
) -> Result<(), String> {
    match value {
        Value::String(_) => scan_string(guardrails, selection, value, is_system, budget, report),
        Value::Array(parts) => {
            for part in parts {
                match part {
                    Value::String(_) => {
                        scan_string(guardrails, selection, part, is_system, budget, report)?;
                    }
                    Value::Object(_) => {
                        if let Some(text) = part.get_mut("text").filter(|t| t.is_string()) {
                            scan_string(guardrails, selection, text, is_system, budget, report)?;
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Scan a single JSON string in place, applying a redaction or surfacing a block.
fn scan_string(
    guardrails: &CompiledGuardrails,
    selection: &RuleSelection,
    value: &mut Value,
    is_system: bool,
    budget: &mut usize,
    report: &mut GuardrailReport,
) -> Result<(), String> {
    let Some(text) = value.as_str() else {
        return Ok(());
    };
    match guardrails.scan_segment_with(text, is_system, budget, report, selection) {
        ScanOutcome::Unchanged => Ok(()),
        ScanOutcome::Redacted(redacted) => {
            *value = Value::String(redacted);
            Ok(())
        }
        ScanOutcome::Blocked(rule) => Err(rule),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rolter_core::{BuiltinRule, GuardAction, GuardStage, GuardrailRule, GuardrailsConfig};
    use serde_json::json;

    fn engine(rules: Vec<GuardrailRule>) -> CompiledGuardrails {
        CompiledGuardrails::from_config(&GuardrailsConfig {
            enabled: true,
            max_scan_bytes: None,
            rules,
        })
    }

    fn block(name: &str, builtin: BuiltinRule) -> GuardrailRule {
        GuardrailRule {
            action: GuardAction::Block,
            ..redact(name, builtin)
        }
    }

    fn selection(engine: &CompiledGuardrails, disable: &[&str], enable: &[&str]) -> RuleSelection {
        engine.resolve_selection(&rolter_core::guardrails::RouteGuardrails {
            disable: disable.iter().map(|s| s.to_string()).collect(),
            enable: enable.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn a_route_override_can_disable_a_redaction_rule() {
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        let body = || json!({"messages": [{"role": "user", "content": "mail me at a@b.com"}]});

        // global behaviour: redacted
        let mut on = body();
        apply_input(
            &g,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut on,
        )
        .unwrap();
        assert!(!on.to_string().contains("a@b.com"), "{on}");

        // the route opts out: the same body passes through untouched
        let mut off = body();
        let report = apply_input(
            &g,
            &selection(&g, &["email"], &[]),
            "/v1/chat/completions",
            &mut off,
        )
        .unwrap();
        assert!(off.to_string().contains("a@b.com"), "{off}");
        assert_eq!(report.redactions, 0);
    }

    #[test]
    fn a_route_override_can_disable_a_blocking_rule() {
        let g = engine(vec![block("card", BuiltinRule::PaymentCard)]);
        let body = || json!({"messages": [{"role": "user", "content": "4111 1111 1111 1111"}]});

        let mut blocked = body();
        assert_eq!(
            apply_input(
                &g,
                &RuleSelection::default(),
                "/v1/chat/completions",
                &mut blocked
            ),
            Err("card".to_string())
        );

        let mut allowed = body();
        assert!(apply_input(
            &g,
            &selection(&g, &["card"], &[]),
            "/v1/chat/completions",
            &mut allowed
        )
        .is_ok());
    }

    #[test]
    fn enable_wins_over_disable_for_the_same_rule() {
        // naming a rule in both is an explicit opt-in, not ambiguous
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        let mut body = json!({"messages": [{"role": "user", "content": "a@b.com"}]});
        apply_input(
            &g,
            &selection(&g, &["email"], &["email"]),
            "/v1/chat/completions",
            &mut body,
        )
        .unwrap();
        assert!(!body.to_string().contains("a@b.com"), "{body}");
    }

    #[test]
    fn disabling_one_rule_leaves_the_others_running() {
        let g = engine(vec![
            redact("email", BuiltinRule::Email),
            block("card", BuiltinRule::PaymentCard),
        ]);
        let mut body =
            json!({"messages": [{"role": "user", "content": "a@b.com and 4111 1111 1111 1111"}]});
        assert_eq!(
            apply_input(
                &g,
                &selection(&g, &["email"], &[]),
                "/v1/chat/completions",
                &mut body
            ),
            Err("card".to_string()),
            "disabling email must not disable card"
        );
    }

    #[test]
    fn a_route_disabling_every_rule_skips_the_scan_entirely() {
        // not just "matches nothing" — the handler must be able to skip the
        // JSON walk, so the engine has to report the route as inactive
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        assert!(g.pre_call_active_for(&RuleSelection::default()));
        assert!(!g.pre_call_active_for(&selection(&g, &["email"], &[])));
    }

    fn redact(name: &str, builtin: BuiltinRule) -> GuardrailRule {
        GuardrailRule {
            name: name.to_string(),
            builtin: Some(builtin),
            pattern: None,
            stage: GuardStage::PreCall,
            action: GuardAction::Redact,
            replacement: None,
            default_on: true,
            include_system: false,
        }
    }

    #[test]
    fn openai_chat_string_content_redacted() {
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "reach me at joe@acme.io"}]
        });
        let report = apply_input(
            &g,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut body,
        )
        .unwrap();
        assert_eq!(report.redactions, 1);
        assert_eq!(
            body["messages"][0]["content"],
            json!("reach me at [REDACTED:EMAIL]")
        );
    }

    #[test]
    fn openai_chat_parts_array_redacted() {
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "ping a@b.com"},
                {"type": "image_url", "image_url": {"url": "http://x"}}
            ]}]
        });
        apply_input(
            &g,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut body,
        )
        .unwrap();
        assert_eq!(
            body["messages"][0]["content"][0]["text"],
            json!("ping [REDACTED:EMAIL]")
        );
    }

    #[test]
    fn system_message_excluded_by_default() {
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "admin ops@corp.com"},
                {"role": "user", "content": "hi user@corp.com"}
            ]
        });
        apply_input(
            &g,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut body,
        )
        .unwrap();
        assert_eq!(body["messages"][0]["content"], json!("admin ops@corp.com"));
        assert_eq!(body["messages"][1]["content"], json!("hi [REDACTED:EMAIL]"));
    }

    #[test]
    fn block_returns_rule_name() {
        let mut rule = redact("card", BuiltinRule::PaymentCard);
        rule.action = GuardAction::Block;
        let g = engine(vec![rule]);
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "card 4111 1111 1111 1111"}]
        });
        let err = apply_input(
            &g,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut body,
        )
        .unwrap_err();
        assert_eq!(err, "card");
    }

    #[test]
    fn anthropic_system_and_messages() {
        let mut rule = redact("email", BuiltinRule::Email);
        rule.include_system = true;
        let g = engine(vec![rule]);
        let mut body = json!({
            "model": "claude-3",
            "system": "route to ops@corp.com",
            "messages": [{"role": "user", "content": "mail me me@x.com"}]
        });
        apply_input(&g, &RuleSelection::default(), "/v1/messages", &mut body).unwrap();
        assert_eq!(body["system"], json!("route to [REDACTED:EMAIL]"));
        assert_eq!(
            body["messages"][0]["content"],
            json!("mail me [REDACTED:EMAIL]")
        );
    }

    #[test]
    fn completions_prompt_redacted() {
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        let mut body = json!({"model": "gpt-4", "prompt": "write to z@z.io"});
        apply_input(&g, &RuleSelection::default(), "/v1/completions", &mut body).unwrap();
        assert_eq!(body["prompt"], json!("write to [REDACTED:EMAIL]"));
    }
}
