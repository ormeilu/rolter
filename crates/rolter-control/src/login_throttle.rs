//! Failed-login throttling for the control plane's password endpoints (#1079).
//!
//! `POST /api/v1/auth/login` used to answer an unlimited number of password
//! guesses at whatever rate the network allowed. argon2id makes each guess
//! expensive, which raises the cost of guessing — and *lowers* the cost of
//! denial of service: an attacker who does not care about the password can
//! saturate the control plane by posting junk credentials, because every one of
//! them buys a full argon2 verification.
//!
//! This module is the counter in front of that work. Two independent subjects
//! are tracked:
//!
//! - the **account** the attempt names, so guessing one password gets slower
//!   and then stops;
//! - the **client address**, so spraying one guess across a thousand accounts
//!   is limited too — a per-account counter alone never fires against that.
//!
//! ## The lock is a clock, not a state
//!
//! A lock is a timestamp with a TTL, never a flag someone has to clear. That is
//! deliberate: a lock an attacker can *set* is a denial-of-service primitive
//! against the operator, since anyone who knows an email address could park the
//! account permanently. Here the worst an attacker achieves is a bounded
//! outage, capped by `max_lock`, that expires on its own.
//!
//! ## Not an enumeration oracle
//!
//! Every key is derived from the *submitted* email, not from a row that was
//! found — an address nobody has registered gets a counter exactly like a real
//! one, is delayed exactly like a real one, and locks exactly like a real one.
//! Together with the constant-cost argon2 verification in
//! [`crate::auth::LocalIdentityProvider::resolve`], the throttle adds no way to
//! tell a wrong password from an unknown account.
//!
//! ## Where the counters live
//!
//! Redis when `--redis-url` is configured, so every control-plane replica
//! shares one view. Without it the counters are in-process, which is correct
//! for the single-node deployment rolter is usually run as and degrades
//! predictably rather than silently: with N replicas and no Redis, an attacker
//! gets N times the budget. That is stated in the docs and warned about at
//! startup rather than left to be discovered.

// the endpoints this guards (`/api/v1/auth/login`, invitation accept) only
// exist in a postgres build, so a default-feature build compiles the throttle
// and never calls it. `cargo hack check --each-feature` runs that build
#![cfg_attr(not(feature = "postgres"), allow(dead_code))]

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

/// Which counter a key belongs to. Kept as a `&'static str` because it is also
/// a metric label, and both uses want a small closed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// the account named by the attempt
    Account,
    /// the network address the attempt came from
    Address,
}

impl Scope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Address => "address",
        }
    }
}

/// One thing being counted: a scope plus an opaque digest of its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Subject {
    scope: Scope,
    /// sha-256 of the identity, so neither Redis nor a heap dump carries the
    /// email address or the client IP in the clear
    digest: String,
}

impl Subject {
    /// The account an attempt names. Lower-cased and trimmed so `Ada@x.io` and
    /// ` ada@x.io ` share one counter rather than doubling an attacker's budget.
    pub(crate) fn account(pepper: &str, email: &str) -> Self {
        Self {
            scope: Scope::Account,
            digest: rolter_auth::hash_key(pepper, &email.trim().to_lowercase()),
        }
    }

    /// The client the attempt came from.
    pub(crate) fn address(pepper: &str, ip: IpAddr) -> Self {
        Self {
            scope: Scope::Address,
            digest: rolter_auth::hash_key(pepper, &ip.to_string()),
        }
    }

    fn key(&self, prefix: &str) -> String {
        format!(
            "rolter:login:{prefix}:{}:{}",
            self.scope.as_str(),
            self.digest
        )
    }

    /// The in-process backend keeps the counter, the lock and the
    /// recently-locked marker in one struct, so it needs one key rather than
    /// the three the redis layout uses.
    fn entry_key(&self) -> String {
        format!("{}:{}", self.scope.as_str(), self.digest)
    }
}

/// Tunables. Every field has a working default; a deployment that configures
/// nothing is still throttled.
#[derive(Clone, Debug)]
pub(crate) struct ThrottleConfig {
    /// failures one account tolerates inside `window` before it locks
    pub account_max_failures: u32,
    /// failures one client address tolerates inside `window` before it locks.
    /// Higher than the account budget on purpose: a shared NAT egress or an
    /// office VPN is one address for a whole floor of legitimate typos
    pub address_max_failures: u32,
    /// how long a counter survives without a new failure
    pub window: Duration,
    /// the first lock; each further lock inside the window doubles it
    pub base_lock: Duration,
    /// ceiling on the doubling, so a lock is always a bounded outage
    pub max_lock: Duration,
    /// the first deliberate delay added to a rejected attempt; doubles with
    /// each failure up to `max_delay`
    pub base_delay: Duration,
    /// ceiling on that delay, so a rejected request cannot pin a connection
    pub max_delay: Duration,
}

impl Default for ThrottleConfig {
    fn default() -> Self {
        Self {
            account_max_failures: 5,
            address_max_failures: 50,
            window: Duration::from_secs(900),
            base_lock: Duration::from_secs(60),
            max_lock: Duration::from_secs(900),
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        }
    }
}

impl ThrottleConfig {
    fn max_failures(&self, scope: Scope) -> u32 {
        match scope {
            Scope::Account => self.account_max_failures,
            Scope::Address => self.address_max_failures,
        }
    }

    /// How long a lock lasts after `failures` failures in this scope. The
    /// first lock is `base_lock`; every failure past the threshold doubles it,
    /// capped at `max_lock`.
    fn lock_for(&self, scope: Scope, failures: u32) -> Duration {
        let over = failures.saturating_sub(self.max_failures(scope));
        let shifted = self
            .base_lock
            .checked_mul(1u32.checked_shl(over.min(20)).unwrap_or(u32::MAX))
            .unwrap_or(self.max_lock);
        shifted.min(self.max_lock)
    }

    /// The delay added to a rejected attempt after `failures` failures.
    fn delay_for(&self, failures: u32) -> Duration {
        let shifted = self
            .base_delay
            .checked_mul(
                1u32.checked_shl(failures.saturating_sub(1).min(20))
                    .unwrap_or(u32::MAX),
            )
            .unwrap_or(self.max_delay);
        shifted.min(self.max_delay)
    }
}

/// What a failed attempt cost.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Penalty {
    /// how long to wait before answering, so guessing gets slower before it
    /// gets refused
    pub delay: Duration,
    /// set when this failure engaged a lock; the value is how long it lasts
    pub locked_for: Option<Duration>,
    /// the scope that locked, for the audit entry and the metric label
    pub locked_scope: Option<Scope>,
}

/// Why a request was refused before any password work happened.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Locked {
    pub scope: Scope,
    pub retry_after: Duration,
}

/// The throttle itself. Cheap to clone; the state is shared.
///
/// The default value counts nothing, which is what a handler test that is not
/// about throttling wants to hold.
#[derive(Clone, Default)]
pub(crate) struct LoginThrottle {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    config: ThrottleConfig,
    pepper: String,
    backend: Backend,
}

enum Backend {
    Redis(redis::Client),
    Memory(Mutex<HashMap<String, Entry>>),
}

/// One in-process counter. `locked_recently` outlives the lock itself so a
/// successful login can say it happened *after* one.
#[derive(Debug)]
struct Entry {
    failures: u32,
    /// when the counter itself lapses
    expires_at: Instant,
    locked_until: Option<Instant>,
    locked_recently: bool,
}

impl LoginThrottle {
    /// A throttle backed by Redis when a client is configured, and by
    /// process-local state otherwise.
    pub(crate) fn new(
        config: ThrottleConfig,
        pepper: String,
        redis: Option<redis::Client>,
    ) -> Self {
        let backend = match redis {
            Some(client) => Backend::Redis(client),
            None => Backend::Memory(Mutex::new(HashMap::new())),
        };
        Self {
            inner: Some(Arc::new(Inner {
                config,
                pepper,
                backend,
            })),
        }
    }

    /// A throttle that counts nothing. Only for deployments that turn it off
    /// explicitly, and for tests of unrelated handlers.
    pub(crate) fn disabled() -> Self {
        Self { inner: None }
    }

    /// The subjects one attempt touches: always the thing it names, plus the
    /// client address when the server could observe one.
    ///
    /// `identity` is whatever the attempt claims to be — an email address on
    /// the login path, an invitation token on the invitation path. It is
    /// deliberately the *submitted* value and never a row that was found, which
    /// is what keeps the throttle from becoming an enumeration oracle.
    pub(crate) fn subjects(&self, identity: &str, client: Option<IpAddr>) -> Vec<Subject> {
        let Some(inner) = &self.inner else {
            return Vec::new();
        };
        let mut out = vec![Subject::account(&inner.pepper, identity)];
        if let Some(ip) = client {
            out.push(Subject::address(&inner.pepper, ip));
        }
        out
    }

    /// Whether any subject is locked right now. Called *before* the password is
    /// verified — refusing after the argon2 work would leave the CPU-exhaustion
    /// half of #1079 wide open.
    pub(crate) async fn check(&self, subjects: &[Subject]) -> Option<Locked> {
        let inner = self.inner.as_ref()?;
        for subject in subjects {
            if let Some(retry_after) = inner.locked_for(subject).await {
                return Some(Locked {
                    scope: subject.scope,
                    retry_after,
                });
            }
        }
        None
    }

    /// Count one failure against every subject and report what it cost.
    pub(crate) async fn record_failure(&self, subjects: &[Subject]) -> Penalty {
        let Some(inner) = &self.inner else {
            return Penalty::default();
        };
        let mut penalty = Penalty::default();
        for subject in subjects {
            let failures = inner.bump(subject).await;
            // the delay follows the account counter: it is the one an ordinary
            // user with a forgotten password moves, and the one that has to
            // stay identical for a registered and an unregistered address
            if subject.scope == Scope::Account {
                penalty.delay = inner.config.delay_for(failures);
            }
            if failures >= inner.config.max_failures(subject.scope) {
                let lock = inner.config.lock_for(subject.scope, failures);
                inner.engage_lock(subject, lock).await;
                // report the longest lock the attempt engaged
                if penalty.locked_for.is_none_or(|current| lock > current) {
                    penalty.locked_for = Some(lock);
                    penalty.locked_scope = Some(subject.scope);
                }
            }
        }
        penalty
    }

    /// Clear the counters after a successful login. Returns whether the account
    /// had been locked inside the current window, so the audit entry can say
    /// the sign-in followed a lockout rather than looking like any other login.
    pub(crate) async fn record_success(&self, subjects: &[Subject]) -> bool {
        let Some(inner) = &self.inner else {
            return false;
        };
        let mut after_lock = false;
        for subject in subjects {
            after_lock |= inner.clear(subject).await;
        }
        after_lock
    }
}

impl Inner {
    async fn locked_for(&self, subject: &Subject) -> Option<Duration> {
        match &self.backend {
            Backend::Redis(client) => {
                let mut conn = client.get_multiplexed_async_connection().await.ok()?;
                // pttl is the remaining lock: storing the deadline and
                // subtracting clocks would make this depend on control-plane and
                // redis clocks agreeing, which across replicas they do not
                let ms: i64 = redis::cmd("PTTL")
                    .arg(subject.key("lock"))
                    .query_async(&mut conn)
                    .await
                    .ok()?;
                (ms > 0).then(|| Duration::from_millis(ms as u64))
            }
            Backend::Memory(map) => {
                let now = Instant::now();
                let guard = map.lock();
                let entry = guard.get(&subject.entry_key())?;
                let until = entry.locked_until?;
                (until > now).then(|| until - now)
            }
        }
    }

    /// Count one failure and return the new total. A backend error counts as
    /// zero: a redis outage must not lock every account in the fleet out, and
    /// it must not fail the login either — the throttle degrades to absent.
    async fn bump(&self, subject: &Subject) -> u32 {
        match &self.backend {
            Backend::Redis(client) => {
                let Ok(mut conn) = client.get_multiplexed_async_connection().await else {
                    return 0;
                };
                let key = subject.key("fail");
                let mut pipe = redis::pipe();
                pipe.atomic()
                    .cmd("INCR")
                    .arg(&key)
                    // refreshed on every failure: the window is "quiet for this
                    // long", not "this long since the first guess", so a slow
                    // drip cannot outlast it
                    .cmd("PEXPIRE")
                    .arg(&key)
                    .arg(self.config.window.as_millis() as u64)
                    .ignore();
                let result: redis::RedisResult<(i64,)> = pipe.query_async(&mut conn).await;
                result.map(|(n,)| n.max(0) as u32).unwrap_or(0)
            }
            Backend::Memory(map) => {
                let now = Instant::now();
                let mut guard = map.lock();
                prune(&mut guard, now);
                let entry = guard.entry(subject.entry_key()).or_insert(Entry {
                    failures: 0,
                    expires_at: now + self.config.window,
                    locked_until: None,
                    locked_recently: false,
                });
                entry.failures += 1;
                entry.expires_at = now + self.config.window;
                entry.failures
            }
        }
    }

    async fn engage_lock(&self, subject: &Subject, lock: Duration) {
        match &self.backend {
            Backend::Redis(client) => {
                let Ok(mut conn) = client.get_multiplexed_async_connection().await else {
                    return;
                };
                let _: redis::RedisResult<()> = redis::pipe()
                    .atomic()
                    .cmd("SET")
                    .arg(subject.key("lock"))
                    .arg(1)
                    .arg("PX")
                    .arg(lock.as_millis() as u64)
                    .ignore()
                    // outlives the lock so the next successful login can report
                    // that it followed one
                    .cmd("SET")
                    .arg(subject.key("locked-recently"))
                    .arg(1)
                    .arg("PX")
                    .arg((lock + self.config.window).as_millis() as u64)
                    .ignore()
                    .query_async(&mut conn)
                    .await;
            }
            Backend::Memory(map) => {
                let now = Instant::now();
                let mut guard = map.lock();
                let entry = guard.entry(subject.entry_key()).or_insert(Entry {
                    failures: 0,
                    expires_at: now + self.config.window,
                    locked_until: None,
                    locked_recently: false,
                });
                entry.locked_until = Some(now + lock);
                entry.locked_recently = true;
                entry.expires_at = now + lock + self.config.window;
            }
        }
    }

    /// Drop every counter for one subject, reporting whether it had been locked
    /// inside the current window.
    async fn clear(&self, subject: &Subject) -> bool {
        match &self.backend {
            Backend::Redis(client) => {
                let Ok(mut conn) = client.get_multiplexed_async_connection().await else {
                    return false;
                };
                let recently: redis::RedisResult<(i64,)> = redis::pipe()
                    .atomic()
                    .cmd("EXISTS")
                    .arg(subject.key("locked-recently"))
                    .cmd("DEL")
                    .arg(subject.key("fail"))
                    .arg(subject.key("lock"))
                    .arg(subject.key("locked-recently"))
                    .ignore()
                    .query_async(&mut conn)
                    .await;
                recently.map(|(n,)| n == 1).unwrap_or(false)
            }
            Backend::Memory(map) => {
                let mut guard = map.lock();
                guard
                    .remove(&subject.entry_key())
                    .is_some_and(|entry| entry.locked_recently)
            }
        }
    }
}

/// The address a request came from, as far as the server can tell.
///
/// The socket peer address is the only value an attacker cannot choose, so it
/// is the default. `X-Forwarded-For` is honoured only when the operator has set
/// `--login-trust-forwarded-for`, because a forwarded header nobody verified is
/// worse than no per-address counter at all: it lets one client rotate through
/// unlimited budgets *and* lets it spend someone else's.
///
/// Behind an un-trusted proxy every client shares the proxy's address, so the
/// per-address budget is a shared one — raise `--login-ip-max-failures` or turn
/// the flag on. The per-account counter is unaffected either way.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientAddr(pub Option<IpAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|info| info.0.ip()),
        ))
    }
}

impl ClientAddr {
    /// Resolve the address to count against, applying the forwarded header only
    /// when the deployment says the hop in front of it is trustworthy.
    pub(crate) fn resolve(
        self,
        headers: &axum::http::HeaderMap,
        trust_forwarded: bool,
    ) -> Option<IpAddr> {
        if trust_forwarded {
            if let Some(first) = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                // leftmost entry is the original client; the rest are proxies
                .and_then(|v| v.split(',').next())
                .map(str::trim)
                .and_then(|v| v.parse::<IpAddr>().ok())
            {
                return Some(first);
            }
        }
        self.0
    }
}

/// Drop lapsed counters so a spraying run cannot grow the map without bound.
/// Called on the failure path, which is the only one that inserts.
fn prune(map: &mut HashMap<String, Entry>, now: Instant) {
    if map.len() < 1024 {
        return;
    }
    map.retain(|_, entry| entry.expires_at > now);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> ThrottleConfig {
        ThrottleConfig {
            account_max_failures: 3,
            address_max_failures: 5,
            window: Duration::from_secs(60),
            base_lock: Duration::from_millis(80),
            max_lock: Duration::from_millis(400),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(8),
        }
    }

    fn throttle() -> LoginThrottle {
        LoginThrottle::new(fast(), "pepper".into(), None)
    }

    #[tokio::test]
    async fn a_clean_account_is_not_locked() {
        let t = throttle();
        let subjects = t.subjects("ada@example.com", None);
        assert!(t.check(&subjects).await.is_none());
    }

    #[tokio::test]
    async fn the_lock_engages_on_the_configured_failure_and_not_before() {
        let t = throttle();
        let subjects = t.subjects("ada@example.com", None);
        for _ in 0..2 {
            assert!(t.record_failure(&subjects).await.locked_for.is_none());
            assert!(t.check(&subjects).await.is_none());
        }
        let penalty = t.record_failure(&subjects).await;
        assert_eq!(penalty.locked_scope, Some(Scope::Account));
        let locked = t.check(&subjects).await.expect("third failure locks");
        assert_eq!(locked.scope, Scope::Account);
    }

    #[tokio::test]
    async fn the_lock_expires_on_its_own() {
        let t = throttle();
        let subjects = t.subjects("ada@example.com", None);
        for _ in 0..3 {
            t.record_failure(&subjects).await;
        }
        assert!(t.check(&subjects).await.is_some());
        // a lock nobody can clear would be a denial-of-service primitive against
        // the operator, so it has to lapse without any request touching it
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(t.check(&subjects).await.is_none());
    }

    #[tokio::test]
    async fn an_unknown_address_is_counted_exactly_like_a_registered_one() {
        // the throttle keys on the submitted email, so it cannot become the
        // enumeration oracle the constant-cost argon2 path avoids being
        let t = throttle();
        let known = t.subjects("ada@example.com", None);
        let unknown = t.subjects("nobody@example.com", None);
        let a = t.record_failure(&known).await;
        let b = t.record_failure(&unknown).await;
        assert_eq!(a, b);
        assert_eq!(a.delay, fast().base_delay);
    }

    #[tokio::test]
    async fn the_delay_grows_with_each_failure_and_stops_at_the_cap() {
        let cfg = fast();
        let t = throttle();
        let subjects = t.subjects("ada@example.com", None);
        let mut seen = Vec::new();
        for _ in 0..6 {
            seen.push(t.record_failure(&subjects).await.delay);
        }
        assert_eq!(
            seen,
            vec![
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(4),
                Duration::from_millis(8),
                cfg.max_delay,
                cfg.max_delay,
            ]
        );
    }

    #[tokio::test]
    async fn each_further_lock_lasts_longer_up_to_the_ceiling() {
        let cfg = fast();
        assert_eq!(cfg.lock_for(Scope::Account, 3), Duration::from_millis(80));
        assert_eq!(cfg.lock_for(Scope::Account, 4), Duration::from_millis(160));
        assert_eq!(cfg.lock_for(Scope::Account, 5), Duration::from_millis(320));
        // and never past the ceiling, however long the run goes on
        assert_eq!(cfg.lock_for(Scope::Account, 40), cfg.max_lock);
    }

    #[tokio::test]
    async fn spraying_one_guess_across_many_accounts_still_locks_the_address() {
        let t = throttle();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        for i in 0..5 {
            let subjects = t.subjects(&format!("victim{i}@example.com"), Some(ip));
            // no account ever reaches its own threshold
            assert!(t.check(&subjects).await.is_none());
            t.record_failure(&subjects).await;
        }
        let next = t.subjects("victim99@example.com", Some(ip));
        let locked = t.check(&next).await.expect("the address budget is spent");
        assert_eq!(locked.scope, Scope::Address);
    }

    #[tokio::test]
    async fn a_successful_login_clears_the_counter_and_reports_a_recent_lock() {
        let t = throttle();
        let subjects = t.subjects("ada@example.com", None);
        for _ in 0..3 {
            t.record_failure(&subjects).await;
        }
        assert!(t.record_success(&subjects).await, "the lock is reported");
        assert!(t.check(&subjects).await.is_none());
        // and the report is not sticky: the next success is an ordinary one
        assert!(!t.record_success(&subjects).await);
    }

    #[tokio::test]
    async fn a_disabled_throttle_counts_nothing() {
        let t = LoginThrottle::disabled();
        let subjects = t.subjects("ada@example.com", None);
        assert!(subjects.is_empty());
        for _ in 0..100 {
            assert_eq!(t.record_failure(&subjects).await, Penalty::default());
        }
        assert!(t.check(&subjects).await.is_none());
    }

    #[test]
    fn a_subject_key_never_carries_the_identity_in_the_clear() {
        let subject = Subject::account("pepper", "ada@example.com");
        let key = subject.key("fail");
        assert!(!key.contains("ada"), "{key}");
        assert!(key.starts_with("rolter:login:fail:account:"));
        // and the same address normalises to one counter
        assert_eq!(subject, Subject::account("pepper", "  Ada@Example.COM "));
    }

    fn headers(forwarded: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-forwarded-for", forwarded.parse().unwrap());
        headers
    }

    #[test]
    fn the_forwarded_client_is_ignored_unless_the_operator_trusts_it() {
        // an unverified forwarded header would hand an attacker an unlimited
        // supply of budgets *and* the ability to spend somebody else's
        let peer: IpAddr = "203.0.113.7".parse().unwrap();
        let addr = ClientAddr(Some(peer));
        assert_eq!(addr.resolve(&headers("198.51.100.9"), false), Some(peer));
        assert_eq!(
            addr.resolve(&headers("198.51.100.9"), true),
            Some("198.51.100.9".parse().unwrap())
        );
    }

    #[test]
    fn a_trusted_forwarded_chain_counts_the_leftmost_client() {
        let addr = ClientAddr(Some("203.0.113.7".parse().unwrap()));
        assert_eq!(
            addr.resolve(&headers(" 198.51.100.9 , 10.0.0.1 , 10.0.0.2"), true),
            Some("198.51.100.9".parse().unwrap())
        );
    }

    #[test]
    fn an_unparseable_forwarded_header_falls_back_to_the_socket_peer() {
        let peer: IpAddr = "203.0.113.7".parse().unwrap();
        let addr = ClientAddr(Some(peer));
        assert_eq!(addr.resolve(&headers("not-an-address"), true), Some(peer));
        // and a request with neither simply has no address subject
        assert_eq!(ClientAddr(None).resolve(&headers("nonsense"), true), None);
    }

    #[test]
    fn the_pepper_separates_two_deployments_sharing_one_redis() {
        assert_ne!(
            Subject::account("one", "ada@example.com"),
            Subject::account("two", "ada@example.com")
        );
    }
}
