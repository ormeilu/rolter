//! The RBAC capability matrix, served from the server's own rules (#534).
//!
//! The dashboard's Roles & Permissions screen rendered a hardcoded matrix, so
//! what it showed and what the control plane enforced could drift silently.
//! This module makes the matrix a server-owned artifact: [`CAPABILITIES`] is
//! the single table both `GET /api/v1/rbac/matrix` (what roles *can* do) and
//! `GET /api/v1/rbac/effective` (what *this caller* can do, at a scope) read
//! from.
//!
//! Nothing here decides an individual request — [`crate::rbac::authorize`]
//! still does that at each handler. The endpoints are read-only and describe
//! the same rule set, so a UI can render and pre-check without ever being
//! trusted: a client that ignores `effective` and calls the endpoint anyway
//! gets the same `403` it always did.

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

/// What a role needs to hold for an action, or that no role suffices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Requirement {
    /// the minimum scoped role that authorizes the action
    Role(Role),
    /// deployment-wide settings only the admin token or a superadmin may touch
    Superadmin,
}

/// One resource and the authority each action on it takes. Read access is a
/// viewer's; mutations are an admin's; deployment-wide policy objects have no
/// scope to be a member of and so are superadmin-only.
struct Capability {
    resource: &'static str,
    /// scope the resource lives under, surfaced so a UI can ask for the right
    /// `org_id`/`team_id`/`project_id` when checking effective permissions
    scope: &'static str,
    read: Requirement,
    write: Requirement,
    /// actions this resource does not support at all (e.g. an audit log is
    /// append-only), reported so a UI does not render a cell that can never
    /// be true for anyone
    unsupported: &'static [Action],
}

const R: Requirement = Requirement::Role(Role::Viewer);
const W: Requirement = Requirement::Role(Role::Admin);
const S: Requirement = Requirement::Superadmin;

/// The capability table. Mirrors the `authorize(...)` calls in
/// [`crate::crud`] and the `require_superadmin` guards on the global policy
/// modules; a resource added there belongs here too.
const CAPABILITIES: &[Capability] = &[
    Capability {
        resource: "org",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[Action::Create],
    },
    Capability {
        resource: "team",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "project",
        scope: "team",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "provider",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "provider_group",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "route",
        scope: "project",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "virtual_key",
        scope: "project",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "budget",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "rate_limit",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "model_price",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "business_unit",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "customer",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "prompt_template",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "skill",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "user",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[],
    },
    Capability {
        resource: "membership",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[Action::Update],
    },
    // listing provisioning tokens is an admin read: the rows name the IdPs a
    // tenant trusts, which is not viewer-grade information
    Capability {
        resource: "scim_token",
        scope: "org",
        read: Requirement::Role(Role::Admin),
        write: W,
        unsupported: &[Action::Update],
    },
    Capability {
        resource: "mcp_server",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[Action::Update],
    },
    // a grant or session belongs to a user: an org admin sees and revokes any,
    // a member only their own, and nobody creates one through the API — they
    // are minted by the OAuth exchange
    Capability {
        resource: "mcp_oauth_grant",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[Action::Create, Action::Update],
    },
    Capability {
        resource: "mcp_oauth_session",
        scope: "org",
        read: R,
        write: W,
        unsupported: &[Action::Create, Action::Update],
    },
    Capability {
        resource: "audit_log",
        scope: "org",
        read: Requirement::Role(Role::Admin),
        write: S,
        unsupported: &[Action::Create, Action::Update, Action::Delete],
    },
    // single sign-on: registering an identity provider or mapping its groups to
    // roles is a way to grant roles, so it needs the same admin bar as granting
    // one directly
    Capability {
        resource: "sso_provider",
        scope: "org",
        read: Requirement::Role(Role::Admin),
        write: Requirement::Role(Role::Admin),
        unsupported: &[Action::Update],
    },
    Capability {
        resource: "sso_group_mapping",
        scope: "org",
        read: Requirement::Role(Role::Admin),
        write: Requirement::Role(Role::Admin),
        unsupported: &[Action::Update],
    },
    Capability {
        resource: "org_auth_policy",
        scope: "org",
        read: Requirement::Role(Role::Admin),
        write: Requirement::Role(Role::Admin),
        unsupported: &[Action::Create, Action::Delete],
    },
    // deployment-wide policy: no tenancy scope exists to be a member of, so
    // these are the admin token's (or a superadmin's) alone
    Capability {
        resource: "feature_flags",
        scope: "deployment",
        read: S,
        write: S,
        unsupported: &[Action::Create, Action::Delete],
    },
    Capability {
        resource: "runtime_policy",
        scope: "deployment",
        read: S,
        write: S,
        unsupported: &[Action::Create, Action::Delete],
    },
    Capability {
        resource: "logging_settings",
        scope: "deployment",
        read: S,
        write: S,
        unsupported: &[Action::Create, Action::Delete],
    },
    Capability {
        resource: "compatibility_policy",
        scope: "deployment",
        read: S,
        write: S,
        unsupported: &[Action::Create, Action::Delete],
    },
    Capability {
        resource: "adaptive_routing_policy",
        scope: "deployment",
        read: S,
        write: S,
        unsupported: &[Action::Create, Action::Delete],
    },
    Capability {
        resource: "cluster_node",
        scope: "deployment",
        read: S,
        write: S,
        unsupported: &[Action::Create],
    },
    Capability {
        resource: "security_settings",
        scope: "deployment",
        read: S,
        write: S,
        unsupported: &[Action::Create, Action::Delete],
    },
];

impl Capability {
    fn requirement(&self, action: Action) -> Option<Requirement> {
        if self.unsupported.contains(&action) {
            return None;
        }
        Some(match action {
            Action::Read => self.read,
            _ => self.write,
        })
    }
}

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
        assert!(!allowed.iter().any(|a| a.ends_with(":delete")));
        // the audit log is an admin read, not a viewer one
        assert!(!allowed.contains(&"audit_log:read".to_string()));
    }

    #[test]
    fn a_member_is_not_yet_a_writer() {
        // member sits between viewer and admin but no CRUD endpoint requires
        // exactly member today; the matrix must say so rather than imply it
        let member = allowed_for(false, Some(Role::Member));
        let viewer = allowed_for(false, Some(Role::Viewer));
        assert_eq!(member, viewer);
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
            // and orgs are created out of band (seeding), not through CRUD
            assert!(!allowed.contains(&"org:create".to_string()));
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
}
