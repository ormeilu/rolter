//! The RBAC capability matrix — the single source of truth for what each
//! guarded route takes (#534, #704).
//!
//! The dashboard's Roles & Permissions screen rendered a hardcoded matrix, so
//! what it showed and what the control plane enforced could drift silently.
//! This module makes the matrix a server-owned artifact: [`CAPABILITIES`] is
//! the one table that backs `GET /api/v1/rbac/matrix` (what roles *can* do),
//! `GET /api/v1/rbac/effective` (what *this caller* can do, at a scope) **and**
//! the guard itself.
//!
//! Every guarded handler names a `(resource, action)` pair through the [`cap!`]
//! macro and hands the resulting [`Requirement`] to [`crate::rbac::authorize`]
//! (or [`crate::rbac::authorize_superadmin`]); no handler names a [`Role`]
//! directly. Because [`requirement_for`] is a `const fn` and [`cap!`] forces it
//! into a `const` item, a pair the table does not define is a **compile
//! error**, and the published matrix is by construction the rule set the guard
//! enforces.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use rolter_auth::Role;
use rolter_store::postgres::repo::MembershipRepo;

use crate::crud::{pool, ApiResult};
use crate::rbac::{resolve_role, role_rank, Principal, ROLES};
use crate::ControlState;

pub(crate) fn router() -> Router<ControlState> {
    Router::new()
        .route("/api/v1/rbac/matrix", get(get_matrix))
        .route("/api/v1/rbac/effective", get(get_effective))
}

/// An action a caller can take on a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Read,
    Create,
    Update,
    Delete,
}

impl Action {
    /// every action, in the order the matrix presents them
    const ALL: [Action; 4] = [Action::Read, Action::Create, Action::Update, Action::Delete];
}

/// What a caller must hold for an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Requirement {
    /// the minimum scoped role that authorizes the action
    Role(Role),
    /// deployment-wide settings only the admin token or a superadmin may touch
    Superadmin,
}

impl Requirement {
    /// const-friendly discriminant test, so [`superadmin_cap!`] can reject a
    /// scoped requirement at compile time
    pub(crate) const fn is_superadmin(self) -> bool {
        matches!(self, Requirement::Superadmin)
    }
}

/// One resource and the authority each action on it takes. `None` means the
/// action does not exist for the resource at all (an audit log is append-only,
/// orgs have no update route), reported so a UI does not render a cell that can
/// never be true for anyone.
struct Capability {
    resource: &'static str,
    /// scope the resource lives under, surfaced so a UI can ask for the right
    /// `org_id`/`team_id`/`project_id` when checking effective permissions
    scope: &'static str,
    read: Option<Requirement>,
    create: Option<Requirement>,
    update: Option<Requirement>,
    delete: Option<Requirement>,
}

const VIEWER: Option<Requirement> = Some(Requirement::Role(Role::Viewer));
const MEMBER: Option<Requirement> = Some(Requirement::Role(Role::Member));
const ADMIN: Option<Requirement> = Some(Requirement::Role(Role::Admin));
const SUPER: Option<Requirement> = Some(Requirement::Superadmin);
/// the resource has no such action
const NA: Option<Requirement> = None;

/// The capability table. Read access is a viewer's, mutations are an admin's,
/// and anything without a tenancy scope to be a member of is superadmin-only.
/// Each entry is what the guard on the corresponding route actually requires —
/// the routes read it from here, so the two cannot disagree.
const CAPABILITIES: &[Capability] = &[
    // an org is created out of band (seeding / the admin token) because there
    // is no wider scope to be an admin of; deleting one is its own admin's
    Capability {
        resource: "org",
        scope: "org",
        read: VIEWER,
        create: SUPER,
        update: NA,
        delete: ADMIN,
    },
    Capability {
        resource: "team",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    Capability {
        resource: "project",
        scope: "team",
        read: VIEWER,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    Capability {
        resource: "provider",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "plugin",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "provider_group",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "route",
        scope: "project",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "virtual_key",
        scope: "project",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    // a key a user mints for themself in a project they belong to: `member`,
    // not `admin`, so read-only viewers still cannot mint one
    Capability {
        resource: "my_virtual_key",
        scope: "project",
        read: NA,
        create: MEMBER,
        update: NA,
        delete: NA,
    },
    Capability {
        resource: "budget",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    Capability {
        resource: "rate_limit",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    // the pricing catalog and the effective model list are global (unscoped),
    // so their mutations are superadmin-only. their reads are not guarded at
    // all today — any authenticated principal may list them — and `viewer` is
    // recorded here as the nominal floor (#766)
    Capability {
        resource: "model_price",
        scope: "deployment",
        read: VIEWER,
        create: NA,
        update: SUPER,
        delete: SUPER,
    },
    Capability {
        resource: "model",
        scope: "deployment",
        read: VIEWER,
        create: NA,
        update: NA,
        delete: SUPER,
    },
    Capability {
        resource: "business_unit",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "customer",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "prompt_template",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "skill",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    // account lifecycle vs. role assignment are split by authority: inviting a
    // user into an org is an org admin's, but editing or deleting the global
    // account (which reaches every org, and can grant the superadmin bit) is
    // superadmin-only
    Capability {
        resource: "user",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: SUPER,
        delete: SUPER,
    },
    Capability {
        resource: "membership",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    // listing provisioning tokens is an admin read: the rows name the IdPs a
    // tenant trusts, which is not viewer-grade information
    Capability {
        resource: "scim_token",
        scope: "org",
        read: ADMIN,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    Capability {
        resource: "mcp_server",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "mcp_tool_group",
        scope: "org",
        read: VIEWER,
        create: ADMIN,
        update: ADMIN,
        delete: ADMIN,
    },
    Capability {
        resource: "mcp_settings",
        scope: "org",
        read: VIEWER,
        create: NA,
        update: ADMIN,
        delete: NA,
    },
    // a grant or session belongs to a user: an org admin sees and revokes any,
    // a member only their own — so the floor is a viewer membership at the org
    // and the handler narrows it to the owner. nobody creates one through the
    // API; they are minted by the OAuth exchange
    Capability {
        resource: "mcp_oauth_grant",
        scope: "org",
        read: VIEWER,
        create: NA,
        update: NA,
        delete: VIEWER,
    },
    Capability {
        resource: "mcp_oauth_session",
        scope: "org",
        read: VIEWER,
        create: NA,
        update: NA,
        delete: VIEWER,
    },
    Capability {
        resource: "audit_log",
        scope: "org",
        read: ADMIN,
        create: NA,
        update: NA,
        delete: NA,
    },
    Capability {
        resource: "invitation",
        scope: "org",
        read: ADMIN,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    // single sign-on: registering an identity provider or mapping its groups to
    // roles is a way to grant roles, so it needs the same admin bar as granting
    // one directly
    Capability {
        resource: "sso_provider",
        scope: "org",
        read: ADMIN,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    Capability {
        resource: "sso_group_mapping",
        scope: "org",
        read: ADMIN,
        create: ADMIN,
        update: NA,
        delete: ADMIN,
    },
    Capability {
        resource: "org_auth_policy",
        scope: "org",
        read: ADMIN,
        create: NA,
        update: ADMIN,
        delete: NA,
    },
    // deployment-wide policy: no tenancy scope exists to be a member of, so
    // these are the admin token's (or a superadmin's) alone
    Capability {
        resource: "feature_flags",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: SUPER,
        delete: NA,
    },
    Capability {
        resource: "runtime_policy",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: SUPER,
        delete: NA,
    },
    Capability {
        resource: "logging_settings",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: SUPER,
        delete: NA,
    },
    Capability {
        resource: "compatibility_policy",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: SUPER,
        delete: NA,
    },
    Capability {
        resource: "adaptive_routing_policy",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: SUPER,
        delete: NA,
    },
    Capability {
        // read-only: the data plane writes it over the internal channel, and
        // no operator role edits a measurement
        resource: "adaptive_routing_telemetry",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: NA,
        delete: NA,
    },
    Capability {
        resource: "cluster_node",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: SUPER,
        delete: SUPER,
    },
    Capability {
        resource: "security_settings",
        scope: "deployment",
        read: SUPER,
        create: NA,
        update: SUPER,
        delete: NA,
    },
    Capability {
        resource: "alert_channel",
        scope: "deployment",
        read: SUPER,
        create: SUPER,
        update: SUPER,
        delete: SUPER,
    },
    Capability {
        resource: "alert_rule",
        scope: "deployment",
        read: SUPER,
        create: SUPER,
        update: SUPER,
        delete: SUPER,
    },
    // notification history is append-only; "create" is asking the deployment to
    // evaluate a rule now, which can emit one
    Capability {
        resource: "alert_history",
        scope: "deployment",
        read: SUPER,
        create: SUPER,
        update: NA,
        delete: NA,
    },
    // MCP tool-call telemetry: written by the gateway, read by an operator
    Capability {
        resource: "mcp_log",
        scope: "deployment",
        read: SUPER,
        create: SUPER,
        update: NA,
        delete: NA,
    },
];

impl Capability {
    const fn requirement(&self, action: Action) -> Option<Requirement> {
        match action {
            Action::Read => self.read,
            Action::Create => self.create,
            Action::Update => self.update,
            Action::Delete => self.delete,
        }
    }
}

/// const-evaluable string equality (`str::eq` is not `const`)
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The requirement the table records for `(resource, action)`.
///
/// A `const fn`, so [`cap!`] turns an unknown resource or an action the
/// resource does not support into a compile error at the call site rather than
/// a guard that silently disagrees with the published matrix.
pub(crate) const fn requirement_for(resource: &str, action: Action) -> Requirement {
    let mut i = 0;
    while i < CAPABILITIES.len() {
        if str_eq(CAPABILITIES[i].resource, resource) {
            return match CAPABILITIES[i].requirement(action) {
                Some(requirement) => requirement,
                None => panic!(
                    "the rbac capability table marks this action unsupported for this resource"
                ),
            };
        }
        i += 1;
    }
    panic!("this resource is not in the rbac capability table (crates/rolter-control/src/rbac_matrix.rs)")
}

/// The [`Requirement`] a guarded route takes, resolved from [`CAPABILITIES`] at
/// compile time: `cap!("provider", Create)`.
macro_rules! cap {
    ($resource:literal, $action:ident) => {{
        const REQUIREMENT: $crate::rbac_matrix::Requirement =
            $crate::rbac_matrix::requirement_for($resource, $crate::rbac_matrix::Action::$action);
        REQUIREMENT
    }};
}

/// Same as [`cap!`], for a guard that has no scope to resolve a role in: also a
/// compile error unless the table says the pair is superadmin-only.
macro_rules! superadmin_cap {
    ($resource:literal, $action:ident) => {{
        const REQUIREMENT: $crate::rbac_matrix::Requirement =
            $crate::rbac_matrix::requirement_for($resource, $crate::rbac_matrix::Action::$action);
        const _: () = assert!(
            REQUIREMENT.is_superadmin(),
            "this guard requires a superadmin but the capability table does not"
        );
        REQUIREMENT
    }};
}

pub(crate) use {cap, superadmin_cap};

#[derive(Debug, Serialize)]
struct ActionView {
    action: Action,
    /// minimum scoped role, absent when the action is superadmin-only
    minimum_role: Option<Role>,
    superadmin_only: bool,
}

#[derive(Debug, Serialize)]
struct ResourceView {
    resource: &'static str,
    scope: &'static str,
    actions: Vec<ActionView>,
}

#[derive(Debug, Serialize)]
struct RoleView {
    role: Role,
    /// total order over roles: viewer `0` < member `1` < admin `2`
    rank: u8,
}

#[derive(Debug, Serialize)]
struct MatrixView {
    roles: Vec<RoleView>,
    resources: Vec<ResourceView>,
}

/// The deployment's capability matrix. Any authenticated principal may read
/// it: it describes the rules, not anyone's access, and a caller learns
/// nothing about a tenant they cannot already see.
async fn get_matrix(_principal: Principal) -> ApiResult<Json<MatrixView>> {
    Ok(Json(MatrixView {
        roles: ROLES
            .iter()
            .map(|&role| RoleView {
                role,
                rank: role_rank(role),
            })
            .collect(),
        resources: CAPABILITIES.iter().map(resource_view).collect(),
    }))
}

fn resource_view(cap: &Capability) -> ResourceView {
    ResourceView {
        resource: cap.resource,
        scope: cap.scope,
        actions: Action::ALL
            .iter()
            .filter_map(|&action| {
                cap.requirement(action).map(|req| ActionView {
                    action,
                    minimum_role: match req {
                        Requirement::Role(role) => Some(role),
                        Requirement::Superadmin => None,
                    },
                    superadmin_only: req == Requirement::Superadmin,
                })
            })
            .collect(),
    }
}

#[derive(Debug, Deserialize)]
struct EffectiveQuery {
    org_id: Option<Uuid>,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct EffectiveView {
    /// true when the caller is the admin token or a superadmin user (which is
    /// also every caller while the control plane runs in open mode)
    superadmin: bool,
    /// the caller's resolved role at the requested scope chain, absent when no
    /// membership reaches it
    role: Option<Role>,
    /// the `resource:action` pairs the caller may perform at that scope
    allowed: Vec<String>,
}

/// What the calling principal may actually do at a scope chain, evaluated
/// server-side from their memberships. A UI uses this to disable controls; the
/// answer is advisory to the client and authoritative only here.
async fn get_effective(
    principal: Principal,
    State(state): State<ControlState>,
    Query(query): Query<EffectiveQuery>,
) -> ApiResult<Json<EffectiveView>> {
    let (superadmin, role) = match &principal {
        Principal::Superadmin => (true, None),
        Principal::User(user) => {
            let memberships = MembershipRepo(pool(&state)).list_for_user(user.id).await?;
            (
                false,
                resolve_role(&memberships, query.org_id, query.team_id, query.project_id),
            )
        }
    };
    Ok(Json(EffectiveView {
        superadmin,
        role,
        allowed: allowed_for(superadmin, role),
    }))
}

/// The `resource:action` pairs a caller with `role` (or superadmin) may
/// perform. Default-deny: a caller with no membership at the scope gets an
/// empty list, exactly as `authorize` would.
fn allowed_for(superadmin: bool, role: Option<Role>) -> Vec<String> {
    let mut allowed = Vec::new();
    for cap in CAPABILITIES {
        for action in Action::ALL {
            let Some(requirement) = cap.requirement(action) else {
                continue;
            };
            let permitted = match requirement {
                Requirement::Superadmin => superadmin,
                Requirement::Role(required) => {
                    superadmin || role.is_some_and(|r| role_rank(r) >= role_rank(required))
                }
            };
            if permitted {
                allowed.push(format!("{}:{}", cap.resource, action_key(action)));
            }
        }
    }
    allowed
}

fn action_key(action: Action) -> &'static str {
    match action {
        Action::Read => "read",
        Action::Create => "create",
        Action::Update => "update",
        Action::Delete => "delete",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_viewer_may_read_everything_scoped_and_write_nothing() {
        let allowed = allowed_for(false, Some(Role::Viewer));
        assert!(allowed.contains(&"provider:read".to_string()));
        assert!(allowed.contains(&"route:read".to_string()));
        assert!(!allowed.iter().any(|a| a.ends_with(":create")));
        // the audit log is an admin read, not a viewer one
        assert!(!allowed.contains(&"audit_log:read".to_string()));
        // the only delete a viewer reaches is revoking their own OAuth grant,
        // which the handler narrows to the owner
        assert_eq!(
            allowed
                .iter()
                .filter(|a| a.ends_with(":delete"))
                .collect::<Vec<_>>(),
            vec!["mcp_oauth_grant:delete", "mcp_oauth_session:delete"],
        );
    }

    #[test]
    fn a_member_may_mint_their_own_key_and_nothing_more() {
        let member = allowed_for(false, Some(Role::Member));
        let viewer = allowed_for(false, Some(Role::Viewer));
        let extra: Vec<_> = member.iter().filter(|a| !viewer.contains(a)).collect();
        assert_eq!(extra, vec!["my_virtual_key:create"]);
    }

    #[test]
    fn an_admin_writes_scoped_resources_but_not_deployment_policy() {
        let allowed = allowed_for(false, Some(Role::Admin));
        assert!(allowed.contains(&"provider:create".to_string()));
        assert!(allowed.contains(&"virtual_key:delete".to_string()));
        assert!(allowed.contains(&"audit_log:read".to_string()));
        // deployment-wide policy has no tenancy scope to be an admin of
        assert!(!allowed.iter().any(|a| a.starts_with("feature_flags:")));
        assert!(!allowed
            .iter()
            .any(|a| a.starts_with("adaptive_routing_policy:")));
        // and neither is the global account lifecycle
        assert!(!allowed.contains(&"user:update".to_string()));
        assert!(!allowed.contains(&"user:delete".to_string()));
    }

    #[test]
    fn no_membership_means_no_permissions() {
        assert!(allowed_for(false, None).is_empty());
    }

    #[test]
    fn superadmin_holds_every_supported_action() {
        let allowed = allowed_for(true, None);
        let supported: usize = CAPABILITIES
            .iter()
            .map(|cap| {
                Action::ALL
                    .iter()
                    .filter(|&&a| cap.requirement(a).is_some())
                    .count()
            })
            .sum();
        assert_eq!(allowed.len(), supported);
    }

    #[test]
    fn unsupported_actions_are_absent_for_everyone() {
        for allowed in [
            allowed_for(true, None),
            allowed_for(false, Some(Role::Admin)),
        ] {
            // an audit log is append-only; nobody deletes one through the API
            assert!(!allowed.contains(&"audit_log:delete".to_string()));
            // and an org has no update route
            assert!(!allowed.contains(&"org:update".to_string()));
        }
    }

    #[test]
    fn the_matrix_lists_every_capability_exactly_once() {
        let mut names: Vec<&str> = CAPABILITIES.iter().map(|c| c.resource).collect();
        names.sort_unstable();
        let mut deduped = names.clone();
        deduped.dedup();
        assert_eq!(names, deduped, "duplicate resource in the capability table");
    }

    // ------------------------------------------------------------------- //
    // drift guards (#704): the table is the only place a requirement lives //
    // ------------------------------------------------------------------- //

    /// Every control-plane module, as `(file name, contents)`. Compiled in, so
    /// the checks below see exactly the source that shipped.
    const MODULES: &[(&str, &str)] = &[
        ("adaptive_policy.rs", include_str!("adaptive_policy.rs")),
        (
            "adaptive_telemetry.rs",
            include_str!("adaptive_telemetry.rs"),
        ),
        ("alerting.rs", include_str!("alerting.rs")),
        ("analytics.rs", include_str!("analytics.rs")),
        ("auth.rs", include_str!("auth.rs")),
        ("auth_policy.rs", include_str!("auth_policy.rs")),
        ("cluster.rs", include_str!("cluster.rs")),
        (
            "compatibility_policy.rs",
            include_str!("compatibility_policy.rs"),
        ),
        ("crud.rs", include_str!("crud.rs")),
        ("feature_flags.rs", include_str!("feature_flags.rs")),
        ("health.rs", include_str!("health.rs")),
        ("invitations.rs", include_str!("invitations.rs")),
        ("lib.rs", include_str!("lib.rs")),
        ("logging_settings.rs", include_str!("logging_settings.rs")),
        ("main.rs", include_str!("main.rs")),
        ("mcp_logs.rs", include_str!("mcp_logs.rs")),
        ("mcp_oauth.rs", include_str!("mcp_oauth.rs")),
        ("me.rs", include_str!("me.rs")),
        ("proxy.rs", include_str!("proxy.rs")),
        ("plugins.rs", include_str!("plugins.rs")),
        ("rbac.rs", include_str!("rbac.rs")),
        ("rbac_matrix.rs", include_str!("rbac_matrix.rs")),
        ("runtime_policy.rs", include_str!("runtime_policy.rs")),
        ("scim.rs", include_str!("scim.rs")),
        ("security.rs", include_str!("security.rs")),
        ("seed.rs", include_str!("seed.rs")),
        ("sso.rs", include_str!("sso.rs")),
    ];

    /// A module the checks below skip, and why.
    const EXEMPT: &[(&str, &str)] = &[
        ("rbac.rs", "owns the role ordering and the guard itself"),
        ("rbac_matrix.rs", "is the capability table"),
        (
            "lib.rs",
            "only enumerates the roles for `GET /api/v1/roles`",
        ),
    ];

    fn is_exempt(name: &str) -> bool {
        EXEMPT.iter().any(|(exempt, _)| *exempt == name)
    }

    /// Whether `source` mentions `Role::` other than as the tail of a longer
    /// path — `DecoratorRole::System` in a seed fixture is a different enum.
    fn names_a_role(source: &str) -> bool {
        source.match_indices("Role::").any(|(at, _)| {
            !source[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    }

    /// The module list must match what is actually on disk, so a new module
    /// cannot quietly escape the checks below.
    #[test]
    fn the_module_list_covers_the_whole_control_plane() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .expect("src/ is readable")
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().to_string_lossy().into_owned();
                name.ends_with(".rs").then_some(name)
            })
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = MODULES.iter().map(|(name, _)| name.to_string()).collect();
        listed.sort();
        assert_eq!(
            listed, on_disk,
            "add the new module to MODULES so its guards are checked",
        );
    }

    /// No handler names a `Role` — every guarded route resolves its requirement
    /// from [`CAPABILITIES`] through `cap!` / `superadmin_cap!`. This is what
    /// makes `GET /api/v1/rbac/matrix` provably the rule set the guard enforces.
    #[test]
    fn no_guard_names_a_role_literal() {
        for (name, source) in MODULES {
            if is_exempt(name) {
                continue;
            }
            assert!(
                !names_a_role(source),
                "{name} names a role directly; use cap!(\"resource\", Action) instead",
            );
        }
    }

    /// And none bypasses the table with a bare superadmin guard.
    #[test]
    fn no_guard_calls_require_superadmin_directly() {
        for (name, source) in MODULES {
            if is_exempt(name) {
                continue;
            }
            assert!(
                !source.contains("require_superadmin("),
                "{name} guards on superadmin directly; use \
                 authorize_superadmin(&principal, superadmin_cap!(...)) instead",
            );
        }
    }

    /// Every row in the table is claimed by at least one guard, so the matrix
    /// cannot publish a resource nothing enforces.
    #[test]
    fn every_capability_is_named_by_a_guard() {
        for capability in CAPABILITIES {
            let scoped = format!("cap!(\"{}\",", capability.resource);
            let named = MODULES
                .iter()
                .filter(|(name, _)| *name != "rbac_matrix.rs")
                .any(|(_, source)| {
                    source
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>()
                        .contains(&scoped)
                });
            assert!(
                named,
                "no guard names '{}'; either guard a route with it or drop the row",
                capability.resource,
            );
        }
    }

    #[test]
    fn the_macro_resolves_the_same_requirement_the_matrix_publishes() {
        assert_eq!(cap!("provider", Read), Requirement::Role(Role::Viewer));
        assert_eq!(cap!("provider", Delete), Requirement::Role(Role::Admin));
        assert_eq!(cap!("feature_flags", Update), Requirement::Superadmin);
        assert_eq!(
            superadmin_cap!("cluster_node", Delete),
            Requirement::Superadmin
        );
    }
}
