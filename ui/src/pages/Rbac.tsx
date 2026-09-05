import { useQuery } from "@tanstack/react-query";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { LoadError } from "@/components/LoadError";
import { PageBody } from "@/components/screen";
import { Skeleton } from "@/components/ui/skeleton";
import {
  fetchMemberships,
  fetchRbacMatrix,
  type RbacAction,
  type RbacCustomRoleView,
  type RbacMatrix,
  type RbacResourceView,
} from "@/lib/api";
import { useScope } from "@/lib/scope";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

// Roles & Permissions, rendered from `GET /api/v1/rbac/matrix` (#1178).
//
// It used to draw a hardcoded 3-role x 7-resource table out of lib/mock.ts,
// which had drifted well away from the ~53-resource CAPABILITIES table the
// control plane actually guards with. That table is the same one the guard
// consults, so a cell here is the rule, not a description of it.

const ACTIONS: RbacAction[] = ["read", "create", "update", "delete"];

// the order CAPABILITIES presents scopes in, widest first. a resource whose
// scope this build does not know about is still rendered, in its own group
const SCOPE_ORDER = ["deployment", "org", "team", "project"];

/** What a single (role, resource, action) cell says. */
type CellState =
  // the action does not exist for this resource at all — an audit log is
  // append-only, orgs have no update route. never "denied": nobody can do it,
  // including a superadmin, because there is no route to call
  | "na"
  // reserved for the deployment admin token or a superadmin account. no org
  // role reaches it, and a custom grant deliberately cannot either
  | "superadmin"
  // the role's rank clears the minimum, or the action needs no membership
  | "allowed"
  // an org-defined role names this exact pair on top of its base role
  | "granted"
  | "denied";

/** A matrix column: one built-in role, or one org-defined role. */
interface RoleColumn {
  key: string;
  label: string;
  rank: number;
  custom?: RbacCustomRoleView;
}

// a cell holds a letter, so `color` is always the -text half of the hue and the
// border keeps the fill. the quiet states carry no `opacity` either: --text-subtle
// at 0.45 is about 2.3:1, and an empty tint already reads as "not granted" (#1181)
const CELL_STYLE: Record<CellState, React.CSSProperties> = {
  allowed: {
    color: "var(--red-folk-text)",
    background: "var(--red-tint)",
    borderColor: "color-mix(in srgb, var(--red-folk) 30%, transparent)",
  },
  granted: {
    color: "var(--status-info-text)",
    background: "rgba(59, 130, 246, .14)",
    borderColor: "color-mix(in srgb, var(--status-info) 34%, transparent)",
  },
  superadmin: {
    color: "var(--status-warning-text)",
    background: "rgba(245, 158, 11, .12)",
    borderColor: "color-mix(in srgb, var(--status-warning) 32%, transparent)",
  },
  denied: {
    color: "var(--text-subtle)",
    background: "transparent",
    borderColor: "var(--border-subtle)",
  },
  na: {
    color: "var(--text-subtle)",
    background: "transparent",
    borderColor: "transparent",
  },
};

const RANK: Record<string, number> = { viewer: 0, member: 1, admin: 2 };

function cellState(
  resource: RbacResourceView,
  action: RbacAction,
  column: RoleColumn,
): CellState {
  const view = resource.actions.find((a) => a.action === action);
  // absent from `actions` means the backend's authority was None
  if (!view) return "na";
  if (view.superadmin_only) return "superadmin";
  if (view.authenticated_only) return "allowed";
  if (view.minimum_role !== null && column.rank >= RANK[view.minimum_role]) {
    return "allowed";
  }
  const granted = column.custom?.grants.some(
    (g) => g.resource === resource.resource && g.action === action,
  );
  return granted ? "granted" : "denied";
}

/** Group resources by scope, widest scope first, keeping table order within. */
function byScope(resources: RbacResourceView[]): [string, RbacResourceView[]][] {
  const groups = new Map<string, RbacResourceView[]>();
  for (const resource of resources) {
    const list = groups.get(resource.scope);
    if (list) list.push(resource);
    else groups.set(resource.scope, [resource]);
  }
  return [...groups.entries()].sort(
    ([a], [b]) =>
      (SCOPE_ORDER.indexOf(a) + 1 || SCOPE_ORDER.length + 1) -
      (SCOPE_ORDER.indexOf(b) + 1 || SCOPE_ORDER.length + 1),
  );
}

function columns(matrix: RbacMatrix): RoleColumn[] {
  const builtins = [...matrix.roles]
    .sort((a, b) => a.rank - b.rank)
    .map((r) => ({ key: r.role, label: r.role, rank: r.rank }));
  const custom = [...matrix.custom_roles]
    .sort((a, b) => a.base_rank - b.base_rank || a.name.localeCompare(b.name))
    .map((r) => ({ key: r.id, label: r.name, rank: r.base_rank, custom: r }));
  return [...builtins, ...custom];
}

export default function Rbac() {
  const { t } = useTranslation();
  const scope = useScope();

  const matrix = useQuery({
    queryKey: ["rbac-matrix", scope.orgId],
    queryFn: () => fetchRbacMatrix(scope.orgId),
    // the built-in half is a compiled-in table and the custom half only moves
    // when someone edits a role, so it never goes stale within a session
    staleTime: Infinity,
    retry: false,
  });

  // membership counts are live and independent: the matrix is still the answer
  // to "what can a role do" even when the user may not list the org's members
  const memberships = useQuery({
    queryKey: ["memberships", scope.orgId],
    queryFn: () => fetchMemberships(scope.orgId as string),
    enabled: !!scope.orgId,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `matrix` is the query the user is actually waiting on for this screen
  useScreenReady(!matrix.isLoading);
  useErrorState(!!matrix.error, "rbac");

  const memberCount = (role: string) =>
    new Set(
      (memberships.data ?? []).filter((m) => m.role === role).map((m) => m.user_id),
    ).size;

  if (matrix.isLoading) return <MatrixSkeleton />;

  if (matrix.error) {
    return (
      <PageBody>
        <LoadError
          error={matrix.error}
          resource={t("errors.resources.rbacMatrix")}
          onRetry={() => matrix.refetch()}
        />
      </PageBody>
    );
  }

  if (!matrix.data) return null;

  const cols = columns(matrix.data);
  const grid = `minmax(200px, 1.6fr) repeat(${cols.length}, minmax(132px, 1fr))`;
  const unknown = matrix.data.custom_roles.flatMap((r) => r.unknown_grants);

  return (
    <PageBody>
      <div className="flex flex-wrap items-start gap-3">
        <p className="max-w-2xl text-sm text-muted-foreground">
          {t("pages.rbac.intro")}
        </p>
        {/* each count is its own node: two plural sentences sharing one text
            node cannot be read back, by a test or by a screen reader */}
        <span className="ml-auto inline-flex flex-none items-center gap-1.5 rounded-full border border-[color:var(--border-subtle)] px-2.5 py-[5px] text-xs text-muted-foreground">
          <span>{t("pages.rbac.roleCount", { count: cols.length })}</span>
          <span aria-hidden>·</span>
          <span>
            {t("pages.rbac.resourceCount", { count: matrix.data.resources.length })}
          </span>
        </span>
      </div>

      {unknown.length > 0 && (
        <p className="rounded-[10px] border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] px-3.5 py-2.5 text-xs text-muted-foreground">
          {t("pages.rbac.unknownGrants", { count: unknown.length })}
        </p>
      )}

      <div tabIndex={0} className="overflow-x-auto focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
        <div className="min-w-[720px] overflow-hidden rounded-[10px] border border-[color:var(--border-subtle)]">
          <div
            className="grid items-end gap-3 border-b border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] px-4 py-[11px]"
            style={{ gridTemplateColumns: grid }}
          >
            <span className="text-[0.6875rem] uppercase tracking-[0.07em] text-[color:var(--text-subtle)]">
              {t("pages.rbac.resourceColumn")}
            </span>
            {cols.map((col) => (
              <div key={col.key} className="flex flex-col gap-0.5">
                <span className="text-sm font-semibold capitalize">{col.label}</span>
                <span className="text-[10px] text-[color:var(--text-subtle)]">
                  {col.custom
                    ? t("pages.rbac.viaAccessProfiles")
                    : memberships.isError
                      ? t("pages.rbac.membersUnknown")
                      : t("pages.rbac.memberCount", { count: memberCount(col.key) })}
                </span>
              </div>
            ))}
          </div>

          {byScope(matrix.data.resources).map(([scopeKey, resources]) => (
            <div key={scopeKey}>
              <div className="border-b border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)]/40 px-4 py-2">
                <span className="text-[0.6875rem] uppercase tracking-[0.07em] text-[color:var(--text-subtle)]">
                  {t(`pages.rbac.scopes.${scopeKey}`, { defaultValue: scopeKey })}
                </span>
              </div>
              {resources.map((resource) => (
                <div
                  key={resource.resource}
                  className="grid items-center gap-3 border-b border-[color:var(--border-subtle)] px-4 py-[11px] last:border-b-0"
                  style={{ gridTemplateColumns: grid }}
                >
                  <span className="truncate font-mono text-xs text-[color:var(--text-secondary)]">
                    {resource.resource}
                  </span>
                  {cols.map((col) => (
                    <div key={col.key} className="flex flex-wrap gap-1">
                      {ACTIONS.map((action) => {
                        const state = cellState(resource, action, col);
                        return (
                          <span
                            key={action}
                            title={t("pages.rbac.cellTitle", {
                              action: t(`pages.rbac.actions.${action}`),
                              state: t(`pages.rbac.states.${state}`),
                            })}
                            className="flex h-5 w-[22px] items-center justify-center rounded-[6px] border font-mono text-[10px] font-semibold"
                            style={CELL_STYLE[state]}
                          >
                            {state === "na" ? "–" : t(`pages.rbac.letters.${action}`)}
                          </span>
                        );
                      })}
                    </div>
                  ))}
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-x-3.5 gap-y-2 text-xs text-[color:var(--text-subtle)]">
        {ACTIONS.map((action) => (
          <span key={action} className="inline-flex items-center gap-[5px]">
            <span
              className="flex h-[18px] w-[18px] items-center justify-center rounded-[6px] border font-mono text-[10px] font-semibold"
              style={CELL_STYLE.allowed}
            >
              {t(`pages.rbac.letters.${action}`)}
            </span>
            {t(`pages.rbac.actions.${action}`)}
          </span>
        ))}
        <span className="inline-flex items-center gap-[5px]">
          <span
            aria-hidden
            className="h-[18px] w-[18px] rounded-[6px] border"
            style={CELL_STYLE.superadmin}
          />
          {t("pages.rbac.legendSuperadmin")}
        </span>
        <span className="inline-flex items-center gap-[5px]">
          <span
            aria-hidden
            className="h-[18px] w-[18px] rounded-[6px] border"
            style={CELL_STYLE.granted}
          />
          {t("pages.rbac.legendGranted")}
        </span>
        <span className="inline-flex items-center gap-[5px]">
          <span
            className="flex h-[18px] w-[18px] items-center justify-center rounded-[6px] border font-mono text-[10px] font-semibold"
            style={CELL_STYLE.na}
          >
            –
          </span>
          {t("pages.rbac.legendNa")}
        </span>
      </div>
    </PageBody>
  );
}

// the matrix is one table, so the placeholder is one table: a header strip and
// a dozen rows, rather than a spinner that says nothing about what is coming
function MatrixSkeleton() {
  return (
    <PageBody>
      <div className="flex items-center gap-3">
        <Skeleton width={320} height={16} />
        <Skeleton width={140} height={22} radius={999} className="ml-auto" />
      </div>
      <div className="overflow-hidden rounded-[10px] border border-[color:var(--border-subtle)]">
        <div className="flex items-center gap-3 border-b border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] px-4 py-[13px]">
          <Skeleton width={90} height={12} />
          <Skeleton width={70} height={12} className="ml-auto" />
          <Skeleton width={70} height={12} />
          <Skeleton width={70} height={12} />
        </div>
        {Array.from({ length: 12 }, (_, i) => (
          <div
            key={i}
            className="flex items-center gap-3 border-b border-[color:var(--border-subtle)] px-4 py-[13px] last:border-b-0"
          >
            <Skeleton width={130} height={12} />
            <Skeleton width={96} height={16} className="ml-auto" />
            <Skeleton width={96} height={16} />
            <Skeleton width={96} height={16} />
          </div>
        ))}
      </div>
    </PageBody>
  );
}
