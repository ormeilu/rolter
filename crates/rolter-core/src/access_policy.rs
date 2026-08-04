//! Model and route visibility carried by an access profile.
//!
//! This lives in `rolter-core` because three planes have to agree on it: the
//! control plane merges the profiles a caller holds, the store resolves that
//! merge onto each virtual key it publishes, and the gateway enforces the
//! result. Two implementations of "deny beats allow" that drift apart is a
//! security bug, so there is exactly one (#791).

use serde::{Deserialize, Serialize};

/// The union of every model/route policy one caller's profiles carry.
///
/// The default is "unrestricted", which is what a deployment with no access
/// profiles has and what every caller without one gets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPolicy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_routes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_routes: Vec<String>,
}

impl ModelPolicy {
    /// No policy at all — the pre-#534 answer.
    pub fn is_unrestricted(&self) -> bool {
        self.allowed_models.is_empty()
            && self.denied_models.is_empty()
            && self.allowed_routes.is_empty()
            && self.denied_routes.is_empty()
    }

    /// Whether the caller may name this upstream model.
    pub fn permits_model(&self, model: &str) -> bool {
        permits(model, &self.allowed_models, &self.denied_models)
    }

    /// Whether the caller may name this configured route.
    pub fn permits_route(&self, route: &str) -> bool {
        permits(route, &self.allowed_routes, &self.denied_routes)
    }

    /// Combine the policies of several profiles the same caller holds.
    ///
    /// Union, not intersection, and for the same reason grants are additive:
    /// two profiles must grant at least what either grants alone, or an
    /// operator would *reduce* someone's access by giving them another
    /// profile. One consequence is load-bearing: a profile that carries no
    /// model restriction at all makes the merged allow-list unrestricted,
    /// because that profile already permitted everything on its own.
    pub fn merge<I>(policies: I) -> Self
    where
        I: IntoIterator<Item = Self>,
    {
        let mut merged = Self::default();
        let mut models_unrestricted = false;
        let mut routes_unrestricted = false;
        for policy in policies {
            if policy.allowed_models.is_empty() {
                models_unrestricted = true;
            }
            if policy.allowed_routes.is_empty() {
                routes_unrestricted = true;
            }
            merged.allowed_models.extend(policy.allowed_models);
            merged.denied_models.extend(policy.denied_models);
            merged.allowed_routes.extend(policy.allowed_routes);
            merged.denied_routes.extend(policy.denied_routes);
        }
        if models_unrestricted {
            merged.allowed_models.clear();
        }
        if routes_unrestricted {
            merged.allowed_routes.clear();
        }
        for list in [
            &mut merged.allowed_models,
            &mut merged.denied_models,
            &mut merged.allowed_routes,
            &mut merged.denied_routes,
        ] {
            list.sort();
            list.dedup();
        }
        merged
    }
}

/// Whether `name` survives an allow/deny pair. An empty allow-list is "no
/// restriction"; a deny always wins. Patterns are exact names or a trailing-`*`
/// prefix glob (`gpt-*`), with a bare `*` meaning everything.
fn permits(name: &str, allowed: &[String], denied: &[String]) -> bool {
    if denied.iter().any(|p| matches_pattern(name, p)) {
        return false;
    }
    allowed.is_empty() || allowed.iter().any(|p| matches_pattern(name, p))
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allowed: &[&str], denied: &[&str]) -> ModelPolicy {
        ModelPolicy {
            allowed_models: allowed.iter().map(|s| s.to_string()).collect(),
            denied_models: denied.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn no_profiles_is_unrestricted() {
        let merged = ModelPolicy::merge([]);
        assert!(merged.is_unrestricted());
        assert!(merged.permits_model("anything"));
        assert!(merged.permits_route("anything"));
    }

    #[test]
    fn allow_list_admits_only_its_own_entries() {
        let merged = ModelPolicy::merge([policy(&["gpt-*", "claude-opus"], &[])]);
        assert!(merged.permits_model("gpt-4o"));
        assert!(merged.permits_model("claude-opus"));
        assert!(!merged.permits_model("claude-haiku"));
    }

    #[test]
    fn deny_beats_allow() {
        let merged = ModelPolicy::merge([policy(&["gpt-*"], &["gpt-4o"])]);
        assert!(merged.permits_model("gpt-4o-mini"));
        assert!(!merged.permits_model("gpt-4o"));

        // a deny-only policy still restricts, and everything else passes
        let deny_only = ModelPolicy::merge([policy(&[], &["secret-*"])]);
        assert!(!deny_only.permits_model("secret-model"));
        assert!(deny_only.permits_model("gpt-4o"));
    }

    #[test]
    fn several_profiles_union_their_allow_lists() {
        let merged = ModelPolicy::merge([policy(&["gpt-*"], &[]), policy(&["claude-*"], &[])]);
        assert!(merged.permits_model("gpt-4o"));
        assert!(merged.permits_model("claude-opus"));
        assert!(!merged.permits_model("llama-3"));
    }

    #[test]
    fn an_unrestricted_profile_lifts_the_others_allow_list() {
        let merged = ModelPolicy::merge([policy(&["gpt-*"], &[]), policy(&[], &[])]);
        assert!(merged.allowed_models.is_empty());
        assert!(merged.permits_model("llama-3"));
    }

    #[test]
    fn a_deny_survives_an_unrestricted_profile() {
        // the allow-list is lifted, but a deny is not: it is the only way to
        // take something away, so union-ing it with "no restriction" must not
        // silently drop it
        let merged = ModelPolicy::merge([policy(&[], &["secret-*"]), policy(&[], &[])]);
        assert!(merged.allowed_models.is_empty());
        assert!(!merged.permits_model("secret-model"));
        assert!(merged.permits_model("gpt-4o"));
    }

    #[test]
    fn bare_star_means_everything() {
        let merged = ModelPolicy::merge([policy(&["*"], &[])]);
        assert!(merged.permits_model("anything-at-all"));
    }

    #[test]
    fn routes_and_models_are_independent_axes() {
        let merged = ModelPolicy::merge([ModelPolicy {
            denied_routes: vec!["internal-*".into()],
            ..Default::default()
        }]);
        assert!(!merged.permits_route("internal-tools"));
        // the model axis carries no restriction of its own
        assert!(merged.permits_model("internal-tools"));
    }

    #[test]
    fn merge_is_deterministic_and_deduplicated() {
        let merged = ModelPolicy::merge([policy(&["b", "a"], &[]), policy(&["a"], &[])]);
        assert_eq!(
            merged.allowed_models,
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
