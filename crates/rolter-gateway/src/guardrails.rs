//! Request- and response-path wiring for the built-in guardrails (ROL-261).
//!
//! Walks the text-bearing fields of an OpenAI- or Anthropic-shaped body and
//! applies the compiled rules for one stage from the snapshot: the request body
//! before proxying (`pre_call`), or a buffered response body before delivering
//! it (`post_call`, #591). Redactions are written back into the JSON in place; a
//! blocking match returns the offending rule name so the handler can reject with
//! an OpenAI-compatible error. The walk only ever passes bounded text slices to
//! the engine, and the returned report carries rule names and counts only —
//! never the matched values.

use rolter_core::guardrails::RuleSelection;
use rolter_core::{CompiledGuardrails, GuardStage, GuardrailReport, ScanOutcome};
use serde_json::Value;

/// One stage's walk over one body: the engine, the route's rule selection, and
/// the mutable scan state that has to be threaded through every nested field.
struct Scan<'a> {
    guardrails: &'a CompiledGuardrails,
    selection: &'a RuleSelection,
    stage: GuardStage,
    budget: usize,
    report: GuardrailReport,
}

impl<'a> Scan<'a> {
    fn new(
        guardrails: &'a CompiledGuardrails,
        selection: &'a RuleSelection,
        stage: GuardStage,
    ) -> Self {
        Self {
            guardrails,
            selection,
            stage,
            budget: guardrails.scan_budget(),
            report: GuardrailReport::default(),
        }
    }

    /// Scan a single JSON string in place, applying a redaction or surfacing a
    /// block.
    fn string(&mut self, value: &mut Value, is_system: bool) -> Result<(), String> {
        let Some(text) = value.as_str() else {
            return Ok(());
        };
        let outcome = match self.stage {
            GuardStage::PreCall => self.guardrails.scan_segment_with(
                text,
                is_system,
                &mut self.budget,
                &mut self.report,
                self.selection,
            ),
            GuardStage::PostCall => self.guardrails.scan_output(
                text,
                &mut self.budget,
                &mut self.report,
                self.selection,
            ),
        };
        match outcome {
            ScanOutcome::Unchanged => Ok(()),
            ScanOutcome::Redacted(redacted) => {
                *value = Value::String(redacted);
                Ok(())
            }
            ScanOutcome::Blocked(rule) => Err(rule),
        }
    }

    /// Scan a `content`/`prompt`/`system` value that may be a string, an array of
    /// strings, or an array of typed parts (`{ "type": "text", "text": "…" }`).
    fn content(&mut self, value: &mut Value, is_system: bool) -> Result<(), String> {
        match value {
            Value::String(_) => self.string(value, is_system),
            Value::Array(parts) => {
                for part in parts {
                    match part {
                        Value::String(_) => self.string(part, is_system)?,
                        Value::Object(_) => {
                            if let Some(text) = part.get_mut("text").filter(|t| t.is_string()) {
                                self.string(text, is_system)?;
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

    /// Scan every message in a `messages` array, treating `system`/`developer`
    /// roles as operator-authored system content.
    fn messages(&mut self, body: &mut Value) -> Result<(), String> {
        let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        for message in messages {
            let is_system = matches!(
                message.get("role").and_then(Value::as_str),
                Some("system") | Some("developer")
            );
            if let Some(content) = message.get_mut("content") {
                self.content(content, is_system)?;
            }
        }
        Ok(())
    }

    /// Scan a named top-level field when the body has it.
    fn field(&mut self, body: &mut Value, name: &str, is_system: bool) -> Result<(), String> {
        match body.get_mut(name) {
            Some(value) => self.content(value, is_system),
            None => Ok(()),
        }
    }

    /// Scan `choices[].message.content` plus the legacy `choices[].text`.
    fn choices(&mut self, body: &mut Value) -> Result<(), String> {
        let Some(choices) = body.get_mut("choices").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        for choice in choices {
            if let Some(content) = choice.pointer_mut("/message/content") {
                self.content(content, false)?;
            }
            if let Some(text) = choice.get_mut("text").filter(|t| t.is_string()) {
                self.string(text, false)?;
            }
        }
        Ok(())
    }
}

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
    let mut scan = Scan::new(guardrails, selection, GuardStage::PreCall);

    match path {
        // OpenAI chat + Responses share the `messages` shape; Responses also
        // accepts a bare `input` string/array
        "/v1/chat/completions" | "/v1/responses" => {
            scan.messages(body)?;
            scan.field(body, "input", false)?;
        }
        // legacy text completions carry a `prompt` string or array of strings
        "/v1/completions" => scan.field(body, "prompt", false)?,
        // Anthropic Messages: a top-level `system` plus user/assistant `messages`
        "/v1/messages" => {
            scan.field(body, "system", true)?;
            scan.messages(body)?;
        }
        _ => {}
    }

    Ok(scan.report)
}

/// Apply post-call guardrails to a buffered, non-streaming response body.
///
/// Same contract as [`apply_input`], one stage later: `Err(rule_name)` means the
/// completion must not be delivered, and a mutated `body` with
/// `report.redactions > 0` must be re-serialized before it goes to the client.
///
/// Only assistant-authored text is walked. Tool-call arguments are left alone:
/// they are a structured payload the client parses, and rewriting a string
/// inside them would hand back arguments that no longer mean what the model
/// intended. An operator who needs those inspected wants the custom-webhook
/// path, which sees the whole envelope.
pub fn apply_output(
    guardrails: &CompiledGuardrails,
    selection: &RuleSelection,
    path: &str,
    body: &mut Value,
) -> Result<GuardrailReport, String> {
    let mut scan = Scan::new(guardrails, selection, GuardStage::PostCall);

    match path {
        // OpenAI chat + legacy completions: one message/text per choice
        "/v1/chat/completions" | "/v1/completions" => scan.choices(body)?,
        // Anthropic Messages replies with a top-level `content` parts array
        "/v1/messages" => scan.field(body, "content", false)?,
        // Responses: typed `output[].content[].text`, plus the flattened
        // `output_text` convenience field when the provider sends it
        "/v1/responses" => {
            if let Some(output) = body.get_mut("output").and_then(Value::as_array_mut) {
                for item in output {
                    if let Some(content) = item.get_mut("content") {
                        scan.content(content, false)?;
                    }
                }
            }
            scan.field(body, "output_text", false)?;
        }
        _ => {}
    }

    Ok(scan.report)
}

/// The output stage resolved on the request path, carried to whichever delivery
/// path ends up serving the response.
///
/// Constructing one is the decision that the response has to be buffered, so
/// [`resolve`](Self::resolve) returns `None` whenever no post-call rule applies —
/// a route without them keeps streaming its non-SSE JSON straight through.
pub struct OutputGuard<'a> {
    guardrails: &'a CompiledGuardrails,
    selection: &'a RuleSelection,
    metrics: &'a crate::metrics::Metrics,
    path: &'a str,
}

/// What the output stage decided about a buffered body.
pub enum OutputDecision {
    /// deliver the bytes as they are
    Unchanged,
    /// deliver these bytes instead
    Masked(bytes::Bytes),
    /// withhold the response; carries the rule name, never the matched text
    Blocked(String),
}

impl<'a> OutputGuard<'a> {
    pub fn resolve(
        guardrails: &'a CompiledGuardrails,
        selection: &'a RuleSelection,
        metrics: &'a crate::metrics::Metrics,
        path: &'a str,
    ) -> Option<Self> {
        guardrails.post_call_active_for(selection).then_some(Self {
            guardrails,
            selection,
            metrics,
            path,
        })
    }

    /// Scan a buffered response body.
    ///
    /// A body that is not JSON is delivered untouched: the stage masks fields of
    /// a known response shape, and there is nothing to walk in an opaque payload.
    /// Callers apply this to successful responses only, so this is not the path
    /// an upstream error body takes.
    pub fn apply(&self, bytes: &bytes::Bytes) -> OutputDecision {
        let Ok(mut parsed) = serde_json::from_slice::<Value>(bytes) else {
            return OutputDecision::Unchanged;
        };
        match apply_output(self.guardrails, self.selection, self.path, &mut parsed) {
            Ok(report) if report.redactions > 0 => {
                self.metrics.guardrail_output_redactions_total.fetch_add(
                    report.redactions as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
                match serde_json::to_vec(&parsed) {
                    Ok(masked) => OutputDecision::Masked(bytes::Bytes::from(masked)),
                    // re-serializing a value that just came out of serde_json
                    // cannot realistically fail, but delivering the unmasked
                    // original is not an acceptable fallback
                    Err(_) => OutputDecision::Blocked("guardrail_serialization".to_string()),
                }
            }
            Ok(_) => OutputDecision::Unchanged,
            Err(rule) => {
                self.metrics
                    .guardrail_output_blocks_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                OutputDecision::Blocked(rule)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rolter_core::{BuiltinRule, GuardAction, GuardrailRule, GuardrailsConfig};
    use serde_json::json;

    fn engine(rules: Vec<GuardrailRule>) -> CompiledGuardrails {
        CompiledGuardrails::from_config(&GuardrailsConfig {
            enabled: true,
            max_scan_bytes: None,
            streaming_post_call: Default::default(),
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

    /// The same rule, moved to the output stage.
    fn out(mut rule: GuardrailRule) -> GuardrailRule {
        rule.stage = GuardStage::PostCall;
        rule
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

    #[test]
    fn openai_chat_completion_content_masked() {
        let g = engine(vec![out(redact("email", BuiltinRule::Email))]);
        let mut body = json!({
            "id": "chatcmpl-1",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "write to ops@corp.com"},
                "finish_reason": "stop"
            }]
        });
        let report = apply_output(
            &g,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut body,
        )
        .unwrap();
        assert_eq!(report.redactions, 1);
        assert_eq!(
            body["choices"][0]["message"]["content"],
            json!("write to [REDACTED:EMAIL]")
        );
    }

    #[test]
    fn a_blocking_output_rule_names_the_rule() {
        let g = engine(vec![out(block("card", BuiltinRule::PaymentCard))]);
        let mut body = json!({
            "choices": [{"message": {"role": "assistant", "content": "4111 1111 1111 1111"}}]
        });
        assert_eq!(
            apply_output(
                &g,
                &RuleSelection::default(),
                "/v1/chat/completions",
                &mut body
            ),
            Err("card".to_string())
        );
    }

    #[test]
    fn anthropic_completion_parts_masked() {
        let g = engine(vec![out(redact("email", BuiltinRule::Email))]);
        let mut body = json!({
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "mail ops@corp.com"},
                {"type": "tool_use", "name": "lookup", "input": {"q": "who@corp.com"}}
            ]
        });
        apply_output(&g, &RuleSelection::default(), "/v1/messages", &mut body).unwrap();
        assert_eq!(body["content"][0]["text"], json!("mail [REDACTED:EMAIL]"));
        // tool-call payloads are structured data the client parses, left intact
        assert_eq!(body["content"][1]["input"]["q"], json!("who@corp.com"));
    }

    #[test]
    fn responses_output_and_output_text_masked() {
        let g = engine(vec![out(redact("email", BuiltinRule::Email))]);
        let mut body = json!({
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "a@b.com"}]}],
            "output_text": "a@b.com"
        });
        apply_output(&g, &RuleSelection::default(), "/v1/responses", &mut body).unwrap();
        assert_eq!(
            body["output"][0]["content"][0]["text"],
            json!("[REDACTED:EMAIL]")
        );
        assert_eq!(body["output_text"], json!("[REDACTED:EMAIL]"));
    }

    #[test]
    fn the_stages_do_not_bleed_into_each_other() {
        // an input rule must not fire on a response body, and vice versa: the
        // stage is the whole point of having two of them
        let g = engine(vec![redact("email", BuiltinRule::Email)]);
        let mut response =
            json!({"choices": [{"message": {"role": "assistant", "content": "a@b.com"}}]});
        apply_output(
            &g,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut response,
        )
        .unwrap();
        assert_eq!(
            response["choices"][0]["message"]["content"],
            json!("a@b.com")
        );
        assert!(!g.post_call_active_for(&RuleSelection::default()));

        let out_only = engine(vec![out(redact("email", BuiltinRule::Email))]);
        let mut request = json!({"messages": [{"role": "user", "content": "a@b.com"}]});
        apply_input(
            &out_only,
            &RuleSelection::default(),
            "/v1/chat/completions",
            &mut request,
        )
        .unwrap();
        assert_eq!(request["messages"][0]["content"], json!("a@b.com"));
        assert!(!out_only.pre_call_active_for(&RuleSelection::default()));
    }

    #[test]
    fn a_route_override_reaches_the_output_stage_too() {
        let g = engine(vec![out(redact("email", BuiltinRule::Email))]);
        let mut body = json!({"choices": [{"message": {"content": "a@b.com"}}]});
        apply_output(
            &g,
            &selection(&g, &["email"], &[]),
            "/v1/chat/completions",
            &mut body,
        )
        .unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], json!("a@b.com"));
        assert!(!g.post_call_active_for(&selection(&g, &["email"], &[])));
    }
}
