//! Local-account login/logout and the `CurrentUser` request extractor (ROL-32).
//!
//! Only mounted when the control plane is started with `--database-url`, same
//! as [`crate::crud`], since these routes need direct pool access.
//!
//! ## Session strategy
//!
//! Sessions are opaque bearer tokens backed by a postgres `sessions` table
//! (`migrations/0013_sessions.sql`), not a stateless JWT. A stateless JWT
//! would need a server-side blocklist to support real logout/revocation
//! before its expiry, which is extra machinery for no benefit here: this
//! deployment already runs postgres for every other auth-adjacent concern
//! (`users`, `memberships`, `virtual_keys` are all postgres rows), so one
//! more table is the smallest addition, not the largest. Redis is already
//! wired into [`crate::ControlState`] for config pub/sub and rate-limit
//! counters, but it's optional (only present when `--redis-url` is set),
//! so making login depend on it would make auth unavailable in postgres-only
//! deployments. The token itself follows the same shape as virtual keys
//! (`rolter_auth::hash_key`/`verify_key`): the plaintext token is returned to
//! the client once and only its peppered SHA-256 digest is stored, so a
//! database leak does not hand out live sessions.
//!
//! `POST /api/v1/auth/logout` deletes the session row outright: revocation is
//! immediate, no blocklist bookkeeping needed.
//!
//! This module builds the `CurrentUser` extractor and proves it works via
//! `GET /api/v1/auth/me`. Wiring role checks into every CRUD mutation is
//! ROL-34, a separate follow-up.

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use async_trait::async_trait;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use rand::Rng;
use rolter_auth::{Credential, Identity, IdentityError, IdentityProvider};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use rolter_store::postgres::models::{Membership, Session, User};
use rolter_store::postgres::repo::{
    AuditLogRepo, MembershipRepo, OrgAuthPolicyRepo, SessionRepo, UserRepo,
};

use crate::ControlState;

/// [`IdentityProvider`] for rolter's own local accounts (email + argon2id
/// password hash). Implements ROL-35: local login is now one of potentially
/// several pluggable providers, alongside [`crate::sso::OidcIdentityProvider`]
/// and, eventually, LDAP (#241).
pub(crate) struct LocalIdentityProvider {
    pool: PgPool,
}

impl LocalIdentityProvider {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IdentityProvider for LocalIdentityProvider {
    fn kind(&self) -> &'static str {
        "local"
    }

    /// Verify an email + password pair against the stored argon2id hash.
    ///
    /// Every rejection path still runs exactly one argon2 verification, so
    /// the response time does not reveal whether the email is registered,
    /// deactivated, or sso-only. This is the same timing-safety property the
    /// pre-ROL-35 inline handler had; refactoring into a provider must not
    /// weaken it, so the "always verify once, decide the error afterward"
    /// structure is preserved exactly.
    async fn resolve(&self, credential: Credential) -> Result<Identity, IdentityError> {
        let Credential::Password { email, password } = credential else {
            return Err(IdentityError::UnsupportedCredential { provider: "local" });
        };
        let email = email.trim();

        // constant hash so an unknown/deactivated/sso-only account still costs
        // one argon2 verification, same as a real one
        const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$csPSM0eDz1Mw8vSYmpUZtA$B00EO0lHN1rK85A5RyDcvLIhc+7tTs0vVoBL4I0MOe0";

        let user_opt = UserRepo(&self.pool)
            .find_by_email(email)
            .await
            .map_err(|e| IdentityError::Provider(e.to_string()))?;

        let mut denied: Option<IdentityError> = None;
        let mut hash_to_check = DUMMY_HASH;
        if let Some(user) = &user_opt {
            if user.deactivated_at.is_some() {
                denied = Some(IdentityError::NotVerified);
            } else if let Some(hash) = &user.password_hash {
                hash_to_check = hash;
                // an org may require its members to come through the IdP; superadmins
                // are exempt on purpose, as the break-glass path back in when the IdP
                // is misconfigured or down
                let blocked = OrgAuthPolicyRepo(&self.pool)
                    .password_login_blocked_for_user(user.id)
                    .await
                    .map_err(|e| IdentityError::Provider(e.to_string()))?;
                if !user.is_superadmin && blocked {
                    denied = Some(IdentityError::PolicyDenied(
                        "password login is disabled for this organization; sign in with sso"
                            .to_string(),
                    ));
                }
            } else {
                // sso-only account (no local password set); reject like a wrong
                // password rather than leaking which accounts exist
                denied = Some(IdentityError::NotVerified);
            }
        }

        let parsed =
            PasswordHash::new(hash_to_check).map_err(|e| IdentityError::Provider(e.to_string()))?;
        let password_verified = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();

        // a policy/status rejection recorded above wins: it ran before the
        // password check in the original sequential form and must keep its
        // more specific status
        if !password_verified {
            denied = denied.or(Some(IdentityError::NotVerified));
        }

        if let Some(err) = denied {
            return Err(err);
        }

        // unreachable with no user: the branch above always records a denial then
        let Some(user) = user_opt else {
            return Err(IdentityError::NotVerified);
        };

        Ok(Identity {
            subject: user.id.to_string(),
            email: user.email.clone(),
            display_name: None,
            groups: Vec::new(),
        })
    }
}

/// how long an issued session stays valid before the client must log in again
const SESSION_TTL_HOURS: i64 = 24 * 7;

pub fn router() -> Router<ControlState> {
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(me))
}

/// Deployment-wide pepper for session tokens (`ROLTER_SESSION_PEPPER`),
/// mirroring `ROLTER_KEY_PEPPER` for virtual keys: tokens are stored as
/// `rolter_auth::hash_key(pepper, token)` so a stolen database dump alone
/// cannot be replayed as a live session.
pub(crate) fn session_pepper() -> String {
    std::env::var("ROLTER_SESSION_PEPPER").unwrap_or_default()
}

fn pool(state: &ControlState) -> &PgPool {
    state
        .pool
        .as_ref()
        .expect("auth router is only mounted when a postgres pool is configured")
}

/// Error type shared by the login/logout/me handlers and the [`CurrentUser`]
/// extractor's rejection.
pub enum AuthError {
    InvalidCredentials,
    Unauthenticated,
    /// the account is real, but its org requires single sign-on (403). Said
    /// plainly rather than as a wrong-password: the user has no way to guess
    /// their way past a policy, and hiding it just sends them to support
    PasswordLoginDisabled,
    /// there is no session *and* the control plane is in open mode, so there
    /// is no account to have a session for. Distinct from
    /// [`Self::Unauthenticated`] because the remedy is completely different:
    /// this one is not fixed by signing in (#942)
    OpenModeNoSession,
    Internal(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        // `code` is the stable, machine-readable half: the dashboard branches on
        // it to explain an unreachable screen rather than rendering a bare
        // failure, and messages stay free to be reworded
        let (status, code, message) = match self {
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "invalid email or password",
            ),
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "missing or invalid session",
            ),
            Self::PasswordLoginDisabled => (
                StatusCode::FORBIDDEN,
                "password_login_disabled",
                "password login is disabled for this organization; sign in with sso",
            ),
            Self::OpenModeNoSession => (
                StatusCode::UNAUTHORIZED,
                "open_mode_no_session",
                "no local account session: this control plane is running in open mode \
                 (no ROLTER_ADMIN_TOKEN), so it has no user accounts to act as. Endpoints \
                 under /api/v1/me/ are per-user and cannot be served without one — set an \
                 admin token and create a local account, or use the admin virtual-key API",
            ),
            Self::Internal(ref msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", msg.as_str())
            }
        };
        (
            status,
            Json(serde_json::json!({"error": {"message": message, "code": code}})),
        )
            .into_response()
    }
}

impl From<rolter_core::Error> for AuthError {
    fn from(err: rolter_core::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

type AuthResult<T> = Result<T, AuthError>;

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    /// bearer token; send as `Authorization: Bearer <token>` on subsequent
    /// requests. Shown once — only its digest is persisted
    token: String,
    expires_at: chrono::DateTime<Utc>,
    user: User,
}

async fn login(
    State(state): State<ControlState>,
    Json(body): Json<LoginRequest>,
) -> AuthResult<Json<LoginResponse>> {
    let email = body.email.trim().to_string();
    let pool = pool(&state);

    let provider = LocalIdentityProvider::new(pool.clone());
    let identity = provider
        .resolve(Credential::Password {
            email,
            password: body.password,
        })
        .await
        .map_err(|err| match err {
            IdentityError::PolicyDenied(_) => AuthError::PasswordLoginDisabled,
            IdentityError::Provider(msg) => AuthError::Internal(msg),
            IdentityError::NotVerified | IdentityError::UnsupportedCredential { .. } => {
                AuthError::InvalidCredentials
            }
        })?;

    let user_id: Uuid = identity
        .subject
        .parse()
        .map_err(|_| AuthError::Internal("resolved identity carried an invalid subject".into()))?;
    let user = UserRepo(pool).get(user_id).await?;

    let (token, token_hash) = generate_session_token(&session_pepper());
    let expires_at = Utc::now() + Duration::hours(SESSION_TTL_HOURS);
    SessionRepo(pool)
        .create(user.id, &token_hash, expires_at)
        .await?;

    // best-effort; login must succeed even if the audit write fails
    let _ = AuditLogRepo(pool)
        .create(
            None,
            Some(user.id),
            "auth.login",
            Some("user"),
            Some(user.id),
            None,
        )
        .await;

    Ok(Json(LoginResponse {
        token,
        expires_at,
        user,
    }))
}

async fn logout(State(state): State<ControlState>, headers: axum::http::HeaderMap) -> StatusCode {
    // no-op if the header is missing or the session is already gone: logout
    // is idempotent from the client's point of view
    if let Some(token) = bearer_token(&headers) {
        let pool = pool(&state);
        let token_hash = rolter_auth::hash_key(&session_pepper(), token);
        if let Ok(Some(session)) = SessionRepo(pool).find_active_by_hash(&token_hash).await {
            let _ = AuditLogRepo(pool)
                .create(
                    None,
                    Some(session.user_id),
                    "auth.logout",
                    Some("user"),
                    Some(session.user_id),
                    None,
                )
                .await;
        }
        let _ = SessionRepo(pool).delete_by_hash(&token_hash).await;
    }
    StatusCode::NO_CONTENT
}

/// extract the bearer token from `Authorization: Bearer <token>`, if present
pub(crate) fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| !t.is_empty())
}

#[derive(Debug, Serialize)]
struct MeResponse {
    user: User,
    memberships: Vec<Membership>,
}

async fn me(
    current: CurrentUser,
    State(state): State<ControlState>,
) -> AuthResult<Json<MeResponse>> {
    let memberships = MembershipRepo(pool(&state))
        .list_for_user(current.user.id)
        .await?;
    Ok(Json(MeResponse {
        user: current.user,
        memberships,
    }))
}

/// generate a fresh opaque session token and its peppered digest; the digest
/// is what's persisted, the token is only ever returned to the client
pub(crate) fn generate_session_token(pepper: &str) -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = format!("rolter_sess_{}", rolter_auth::hex_encode(&bytes));
    let hash = rolter_auth::hash_key(pepper, &token);
    (token, hash)
}

/// The authenticated user resolved from `Authorization: Bearer <token>` (a
/// live, unexpired [`Session`] row). Extracting this on a handler is enough
/// to require login; per-role authorization on top of it is ROL-34.
///
/// ```ignore
/// async fn protected(current: CurrentUser) -> Json<User> {
///     Json(current.user)
/// }
/// ```
pub struct CurrentUser {
    pub user: User,
    #[allow(dead_code)] // not consumed yet; kept for ROL-34's role checks
    pub session: Session,
}

impl FromRequestParts<ControlState> for CurrentUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ControlState,
    ) -> Result<Self, Self::Rejection> {
        // in open mode the admin `Principal` extractor passes every request as
        // superadmin, so admin routes work while these ones 401 — which reads
        // as "my login is broken" rather than "this endpoint needs an account
        // this deployment does not have" (#942). Same fact both extractors
        // branch on, so the two can never disagree about what open mode is
        let open_mode = state.admin_token.is_none();
        let unauthenticated = || {
            if open_mode {
                AuthError::OpenModeNoSession
            } else {
                AuthError::Unauthenticated
            }
        };

        let token = bearer_token(&parts.headers).ok_or_else(unauthenticated)?;

        let token_hash = rolter_auth::hash_key(&session_pepper(), token);
        let pool = pool(state);
        let session = SessionRepo(pool)
            .find_active_by_hash(&token_hash)
            .await
            .map_err(AuthError::from)?
            .ok_or_else(unauthenticated)?;
        let user = UserRepo(pool)
            .get(session.user_id)
            .await
            .map_err(AuthError::from)?;
        Ok(CurrentUser { user, session })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::rand_core::OsRng;
    use argon2::password_hash::{PasswordHasher, SaltString};

    fn hash_password(password: &str) -> String {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    #[test]
    fn password_hash_round_trips() {
        let hash = hash_password("correct horse battery staple");
        let parsed = PasswordHash::new(&hash).unwrap();
        assert!(Argon2::default()
            .verify_password(b"correct horse battery staple", &parsed)
            .is_ok());
    }

    #[test]
    fn wrong_password_is_rejected() {
        let hash = hash_password("correct horse battery staple");
        let parsed = PasswordHash::new(&hash).unwrap();
        assert!(Argon2::default()
            .verify_password(b"wrong password", &parsed)
            .is_err());
    }

    #[test]
    fn session_token_hash_round_trips_and_is_peppered() {
        let (token, hash) = generate_session_token("pepper");
        assert!(token.starts_with("rolter_sess_"));
        // 12 chars prefix + 64 chars hex (32 bytes) = 76 chars
        assert_eq!(token.len(), 76);
        assert!(token["rolter_sess_".len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
        // the same token under the same pepper always re-hashes to the
        // stored digest, which is how session lookup matches it
        assert_eq!(rolter_auth::hash_key("pepper", &token), hash);
        // a different pepper yields a different digest, same as virtual keys
        assert_ne!(rolter_auth::hash_key("other", &token), hash);
    }

    #[test]
    fn hex_encode_correctness() {
        let bytes = [0xde, 0xad, 0xbe, 0xef, 0x00, 0xff, 0x01, 0x0a];
        let encoded = rolter_auth::hex_encode(&bytes);
        assert_eq!(encoded, "deadbeef00ff010a");
    }

    #[test]
    fn session_tokens_are_unique() {
        let (a, _) = generate_session_token("pepper");
        let (b, _) = generate_session_token("pepper");
        assert_ne!(a, b);
    }

    /// Read the `(status, code, message)` an [`AuthError`] renders as.
    async fn rendered(err: AuthError) -> (StatusCode, String, String) {
        let response = err.into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (
            status,
            json["error"]["code"].as_str().unwrap().to_string(),
            json["error"]["message"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test]
    async fn every_auth_error_carries_a_machine_readable_code() {
        // the dashboard branches on `code` to explain a screen it cannot serve;
        // a variant that renders without one degrades to a bare failure
        for err in [
            AuthError::InvalidCredentials,
            AuthError::Unauthenticated,
            AuthError::PasswordLoginDisabled,
            AuthError::OpenModeNoSession,
            AuthError::Internal("boom".to_string()),
        ] {
            let (_, code, message) = rendered(err).await;
            assert!(!code.is_empty());
            assert!(!message.is_empty());
        }
    }

    #[tokio::test]
    async fn open_mode_is_distinguishable_from_a_missing_session() {
        // both are 401, and conflating them is the whole bug (#942): one is
        // fixed by signing in, the other cannot be
        let (plain_status, plain_code, _) = rendered(AuthError::Unauthenticated).await;
        let (open_status, open_code, open_message) = rendered(AuthError::OpenModeNoSession).await;

        assert_eq!(plain_status, StatusCode::UNAUTHORIZED);
        assert_eq!(open_status, StatusCode::UNAUTHORIZED);
        assert_ne!(plain_code, open_code);
        assert_eq!(open_code, "open_mode_no_session");

        // the message has to name the cause and the remedy, or the dashboard is
        // left saying "unauthorized" in more words
        assert!(open_message.contains("open mode"), "{open_message}");
        assert!(
            open_message.contains("ROLTER_ADMIN_TOKEN"),
            "{open_message}"
        );
    }
}
