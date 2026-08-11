//! `rolter check` — pre-boot validation for a production deployment (#854).
//!
//! The local path (`rolter easy-up`) is deliberately frictionless and may stay
//! loose: it runs with no database, no provider keys and no secrets at all.
//! That is a feature, and this command must never make it prompt for anything.
//!
//! The production path had none of the guardrails that made the local looseness
//! safe. A misconfigured deployment did not fail — it *started degraded*. The
//! sharpest example, and the reason this exists: when the KEK is missing, the
//! control plane's default-provider seed logs a warning and carries on with an
//! **unsealed** inline `api_key`. Nothing fails, nothing is red, and a
//! credential the operator believed was encrypted at rest is not.
//!
//! So this is a gate, not a linter: it exits non-zero on anything that would
//! produce a silently-degraded production process, and it is meant to run
//! before the process starts — as a container entrypoint step, a Helm hook, or
//! by hand.

use std::fmt::Write as _;

use clap::Args;

/// Environment variable holding the key-encryption key.
///
/// Duplicated from `rolter_store::postgres::crypto::KEK_ENV` rather than
/// imported: that crate is optional here (behind the `postgres` feature) and
/// `rolter check` has to work without it. A `#[cfg(feature = "postgres")]` test
/// below pins the two together — given that this whole command exists because
/// the *name* of this variable drifted between code and docs, letting it drift
/// again inside the codebase would be a poor joke.
const KEK_ENV: &str = "ROLTER_KEK";

/// Variable name several docs pages wrongly told operators to set. Nothing in
/// the codebase has ever read it, so a deployment that sets it has no KEK at
/// all — the single most consequential way to end up silently degraded.
const WRONG_KEK_ENV: &str = "ROLTER_MASTER_KEY";

/// Credentials that ship in the example files. Finding one in a production
/// environment means a placeholder was carried through rather than replaced.
const EXAMPLE_VALUES: &[(&str, &str)] = &[
    (
        "postgres://rolter:rolter@localhost:5432/rolter",
        "the example database URL, credentials and all",
    ),
    (
        "cm9sdGVyLWUyZS10ZXN0LWtlay1ub3Qtc2VjcmV0ISE=",
        "the throwaway KEK from the e2e test harness",
    ),
];

/// Shortest KEK we accept. The KEK is stretched through SHA-256, so any string
/// *works*; that is exactly why a length floor is worth enforcing here rather
/// than trusting the derivation to disguise a weak secret.
const MIN_KEK_LEN: usize = 16;

#[derive(Args, Debug, Default)]
pub struct CheckArgs {
    /// path to the gateway config file to validate
    #[arg(short, long, env = "ROLTER_CONFIG")]
    pub config: Option<String>,

    /// treat warnings as failures too
    #[arg(long)]
    pub strict: bool,
}

/// One finding. `fatal` decides the exit code; a warning is printed and the
/// run still succeeds unless `--strict`.
#[derive(Debug, PartialEq, Eq)]
struct Finding {
    fatal: bool,
    title: String,
    detail: String,
}

impl Finding {
    fn error(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            fatal: true,
            title: title.into(),
            detail: detail.into(),
        }
    }

    fn warn(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            fatal: false,
            title: title.into(),
            detail: detail.into(),
        }
    }
}

/// How the checks read the environment. Taking this as a trait keeps every
/// rule unit-testable without mutating real process env, which is global state
/// and races across parallel tests.
trait Env {
    fn get(&self, key: &str) -> Option<String>;
}

struct ProcessEnv;

impl Env for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.trim().is_empty())
    }
}

/// The KEK rules. This is the check that motivated the command.
fn check_kek(env: &dyn Env, out: &mut Vec<Finding>) {
    match env.get(KEK_ENV) {
        None => {
            // name the likely cause rather than only the symptom: several docs
            // pages told operators to set the wrong variable
            let detail = if env.get(WRONG_KEK_ENV).is_some() {
                format!(
                    "{WRONG_KEK_ENV} is set but nothing reads it — the variable rolter reads is \
                     {KEK_ENV}. Rename it. Until you do, provider credentials are not encrypted \
                     at rest: the control plane logs a warning and stores an inline api_key \
                     unsealed rather than failing."
                )
            } else {
                format!(
                    "{KEK_ENV} is unset. Provider credentials cannot be encrypted at rest, and \
                     the default-provider seed will store an inline api_key unsealed with only a \
                     warning. Generate one with `openssl rand -hex 32`."
                )
            };
            out.push(Finding::error(format!("{KEK_ENV} is missing"), detail));
        }
        Some(kek) if kek.trim().len() < MIN_KEK_LEN => out.push(Finding::error(
            format!("{KEK_ENV} is too short"),
            format!(
                "the KEK is {} characters; use at least {MIN_KEK_LEN}. It is stretched through \
                 SHA-256, so a short secret still produces a valid key — and a brute-forceable \
                 one. Generate with `openssl rand -hex 32`.",
                kek.trim().len()
            ),
        )),
        Some(_) => {}
    }
}

/// Management API and snapshot endpoint must not be open in production.
fn check_admin_token(env: &dyn Env, out: &mut Vec<Finding>) {
    if env.get("ROLTER_ADMIN_TOKEN").is_none() {
        out.push(Finding::error(
            "ROLTER_ADMIN_TOKEN is missing",
            "the management API and /internal/snapshot are unauthenticated without it. Anyone \
             who can reach the control plane can read the whole config snapshot and mutate \
             providers, routes and virtual keys.",
        ));
    }
}

/// A production control plane without a database is not a production control
/// plane: config, RBAC, keys and pricing all live in Postgres.
fn check_datastores(env: &dyn Env, out: &mut Vec<Finding>) {
    match env.get("ROLTER_DATABASE_URL") {
        None => out.push(Finding::error(
            "ROLTER_DATABASE_URL is missing",
            "the control plane falls back to an in-memory store, so every provider, route, \
             virtual key and budget is lost on restart.",
        )),
        Some(url) if !url.starts_with("postgres://") && !url.starts_with("postgresql://") => out
            .push(Finding::error(
                "ROLTER_DATABASE_URL is not a postgres URL",
                format!(
                    "expected a postgres:// or postgresql:// URL, got `{}`",
                    redact(&url)
                ),
            )),
        Some(_) => {}
    }

    if env.get("ROLTER_REDIS_URL").is_none() {
        out.push(Finding::warn(
            "ROLTER_REDIS_URL is unset",
            "rate-limit counters and spend budgets fall back to per-process state, so limits are \
             enforced per replica rather than per deployment. Fine for a single replica, wrong \
             for more than one.",
        ));
    }
}

/// Placeholder values that were meant to be replaced.
fn check_example_values(env: &dyn Env, out: &mut Vec<Finding>) {
    for var in [
        KEK_ENV,
        "ROLTER_DATABASE_URL",
        "ROLTER_ADMIN_TOKEN",
        "ROLTER_INTERNAL_TOKEN",
    ] {
        let Some(value) = env.get(var) else { continue };
        for (example, what) in EXAMPLE_VALUES {
            if value.contains(example) {
                out.push(Finding::error(
                    format!("{var} still holds an example value"),
                    format!(
                        "this is {what}. It is published in the repository, so treat it as \
                             public knowledge and replace it."
                    ),
                ));
            }
        }
    }
}

/// Binding the management plane to every interface is the one network default
/// that is right for local and wrong for production.
fn check_exposure(env: &dyn Env, out: &mut Vec<Finding>) {
    if env
        .get("ROLTER_CONTROL_HOST")
        .is_none_or(|h| h == "0.0.0.0")
    {
        out.push(Finding::warn(
            "control plane binds 0.0.0.0",
            "the management API is reachable on every interface. Bind it to a private address, \
             or keep it behind an ingress that terminates TLS and restricts access.",
        ));
    }
}

/// Redact anything that looks like credentials in a URL before printing it —
/// this output goes to CI logs.
fn redact(url: &str) -> String {
    match (url.find("://"), url.find('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            format!("{}://***@{}", &url[..scheme], &url[at + 1..])
        }
        _ => url.to_string(),
    }
}

fn run_checks(env: &dyn Env) -> Vec<Finding> {
    let mut out = Vec::new();
    check_kek(env, &mut out);
    check_admin_token(env, &mut out);
    check_datastores(env, &mut out);
    check_example_values(env, &mut out);
    check_exposure(env, &mut out);
    out
}

/// Render findings into a report. Returns `(report, should_fail)`.
fn report(findings: &[Finding], strict: bool) -> (String, bool) {
    let errors = findings.iter().filter(|f| f.fatal).count();
    let warnings = findings.len() - errors;

    let mut out = String::new();
    for finding in findings {
        let tag = if finding.fatal { "error" } else { "warn " };
        let _ = writeln!(out, "{tag}  {}", finding.title);
        let _ = writeln!(out, "       {}\n", finding.detail);
    }

    if findings.is_empty() {
        out.push_str("all pre-boot checks passed\n");
        return (out, false);
    }

    let _ = writeln!(out, "{errors} error(s), {warnings} warning(s)");
    (out, errors > 0 || (strict && warnings > 0))
}

pub async fn run(args: CheckArgs) -> anyhow::Result<()> {
    let mut findings = run_checks(&ProcessEnv);

    // the config file is optional: a fully DB-backed deployment has none
    if let Some(path) = args.config.as_deref() {
        if let Err(error) = rolter_core::GatewayConfig::load(std::path::Path::new(path)) {
            findings.push(Finding::error(
                format!("config file {path} is not usable"),
                error.to_string(),
            ));
        }
    }

    let (text, failed) = report(&findings, args.strict);
    print!("{text}");
    if failed {
        anyhow::bail!("pre-boot validation failed; refusing to report this deployment as ready");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeEnv(HashMap<String, String>);

    impl FakeEnv {
        fn with(mut self, key: &str, value: &str) -> Self {
            self.0.insert(key.into(), value.into());
            self
        }

        /// a deployment that passes every check, as the baseline to perturb
        fn healthy() -> Self {
            Self::default()
                .with(KEK_ENV, "b6f1c0d2e3a4957886b1c0d2e3a49578")
                .with("ROLTER_ADMIN_TOKEN", "an-admin-token")
                .with("ROLTER_DATABASE_URL", "postgres://user:pw@db:5432/rolter")
                .with("ROLTER_REDIS_URL", "redis://redis:6379")
                .with("ROLTER_CONTROL_HOST", "127.0.0.1")
        }
    }

    impl Env for FakeEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned().filter(|v| !v.trim().is_empty())
        }
    }

    fn titles(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.title.as_str()).collect()
    }

    #[test]
    fn a_correctly_configured_deployment_is_clean() {
        let findings = run_checks(&FakeEnv::healthy());
        assert!(
            findings.is_empty(),
            "unexpected findings: {:?}",
            titles(&findings)
        );
        let (text, failed) = report(&findings, true);
        assert!(!failed);
        assert!(text.contains("all pre-boot checks passed"));
    }

    #[test]
    fn a_missing_kek_is_fatal() {
        let mut env = FakeEnv::healthy();
        env.0.remove(KEK_ENV);
        let findings = run_checks(&env);
        let kek = findings
            .iter()
            .find(|f| f.title.contains(KEK_ENV))
            .expect("missing KEK not reported");
        assert!(kek.fatal);
        // the consequence, not just the absence
        assert!(kek.detail.contains("unsealed"));
    }

    #[test]
    fn the_wrong_kek_variable_is_named_explicitly() {
        // the exact trap the docs led operators into: ROLTER_MASTER_KEY is set,
        // looks configured, and is read by nothing
        let mut env = FakeEnv::healthy();
        env.0.remove(KEK_ENV);
        let env = env.with(WRONG_KEK_ENV, "a-perfectly-good-secret-value");
        let findings = run_checks(&env);
        let kek = findings
            .iter()
            .find(|f| f.title.contains(KEK_ENV))
            .expect("missing KEK not reported");
        assert!(
            kek.detail.contains(WRONG_KEK_ENV) && kek.detail.contains("nothing reads it"),
            "the report must name the wrong variable: {}",
            kek.detail
        );
    }

    #[test]
    fn a_short_kek_is_rejected_despite_hashing() {
        let env = FakeEnv::healthy().with(KEK_ENV, "hunter2");
        let findings = run_checks(&env);
        assert!(findings
            .iter()
            .any(|f| f.fatal && f.title.contains("too short")));
    }

    #[test]
    fn example_values_are_caught() {
        let env = FakeEnv::healthy().with(
            "ROLTER_DATABASE_URL",
            "postgres://rolter:rolter@localhost:5432/rolter",
        );
        let findings = run_checks(&env);
        assert!(findings
            .iter()
            .any(|f| f.fatal && f.title.contains("example value")));
    }

    #[test]
    fn the_e2e_throwaway_kek_is_caught() {
        let env = FakeEnv::healthy().with(KEK_ENV, "cm9sdGVyLWUyZS10ZXN0LWtlay1ub3Qtc2VjcmV0ISE=");
        let findings = run_checks(&env);
        assert!(findings
            .iter()
            .any(|f| f.fatal && f.title.contains("example value")));
    }

    #[test]
    fn a_missing_admin_token_is_fatal() {
        let mut env = FakeEnv::healthy();
        env.0.remove("ROLTER_ADMIN_TOKEN");
        let findings = run_checks(&env);
        assert!(findings
            .iter()
            .any(|f| f.fatal && f.title.contains("ROLTER_ADMIN_TOKEN")));
    }

    #[test]
    fn a_missing_database_is_fatal_and_a_missing_redis_is_only_a_warning() {
        let mut env = FakeEnv::healthy();
        env.0.remove("ROLTER_DATABASE_URL");
        env.0.remove("ROLTER_REDIS_URL");
        let findings = run_checks(&env);

        let db = findings
            .iter()
            .find(|f| f.title.contains("ROLTER_DATABASE_URL"))
            .expect("missing database not reported");
        assert!(db.fatal);

        let redis = findings
            .iter()
            .find(|f| f.title.contains("ROLTER_REDIS_URL"))
            .expect("missing redis not reported");
        // a single-replica deployment is legitimately fine without redis
        assert!(!redis.fatal);
    }

    #[test]
    fn a_non_postgres_database_url_is_rejected() {
        let env = FakeEnv::healthy().with("ROLTER_DATABASE_URL", "mysql://user:pw@db/rolter");
        let findings = run_checks(&env);
        assert!(findings
            .iter()
            .any(|f| f.fatal && f.title.contains("not a postgres URL")));
    }

    #[test]
    fn a_rejected_database_url_is_redacted_in_the_report() {
        let env = FakeEnv::healthy().with("ROLTER_DATABASE_URL", "mysql://user:hunter2@db/rolter");
        let findings = run_checks(&env);
        let (text, _) = report(&findings, false);
        assert!(
            !text.contains("hunter2"),
            "credentials leaked into the report:\n{text}"
        );
        assert!(text.contains("***@db/rolter"));
    }

    #[test]
    fn binding_the_control_plane_to_all_interfaces_warns() {
        let env = FakeEnv::healthy().with("ROLTER_CONTROL_HOST", "0.0.0.0");
        let findings = run_checks(&env);
        let exposure = findings
            .iter()
            .find(|f| f.title.contains("0.0.0.0"))
            .expect("exposure not reported");
        assert!(!exposure.fatal);
    }

    #[test]
    fn an_unset_control_host_warns_because_the_default_is_all_interfaces() {
        let mut env = FakeEnv::healthy();
        env.0.remove("ROLTER_CONTROL_HOST");
        let findings = run_checks(&env);
        assert!(findings.iter().any(|f| f.title.contains("0.0.0.0")));
    }

    #[test]
    fn strict_promotes_warnings_to_failure() {
        let env = FakeEnv::healthy().with("ROLTER_CONTROL_HOST", "0.0.0.0");
        let findings = run_checks(&env);
        assert!(findings.iter().all(|f| !f.fatal), "expected warnings only");
        assert!(!report(&findings, false).1, "warnings alone must not fail");
        assert!(report(&findings, true).1, "--strict must fail on warnings");
    }

    #[test]
    fn blank_values_count_as_unset() {
        // `ROLTER_KEK=` in a compose file is absence, not configuration
        let env = FakeEnv::healthy().with(KEK_ENV, "   ");
        let findings = run_checks(&env);
        assert!(findings
            .iter()
            .any(|f| f.fatal && f.title.contains("is missing")));
    }

    /// The command exists because this variable's name drifted between code and
    /// docs. Pin the local copy to the store's definition so it cannot drift
    /// again inside the codebase.
    #[cfg(feature = "postgres")]
    #[test]
    fn the_kek_variable_name_matches_the_store() {
        assert_eq!(KEK_ENV, rolter_store::postgres::crypto::KEK_ENV);
    }

    #[test]
    fn redact_leaves_a_credential_free_url_alone() {
        assert_eq!(
            redact("postgres://db:5432/rolter"),
            "postgres://db:5432/rolter"
        );
        assert_eq!(redact("not-a-url"), "not-a-url");
    }
}
