import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Shield, ShieldOff } from "lucide-react";
import * as React from "react";

import {
  ListHeader,
  ListRow,
  ListTable,
  PageBody,
  Pill,
  RowIconButton,
} from "@/components/screen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import {
  fetchMcpGrants,
  fetchMcpServers,
  fetchMcpSessions,
  fetchUsers,
  revokeMcpGrant,
  revokeMcpSession,
  type McpOAuthGrantRow,
  type McpOAuthSessionRow,
  type McpServerRow,
  type UserRow,
} from "@/lib/api";
import { useScope } from "@/lib/scope";

// both screens read crates/rolter-control/src/mcp_oauth.rs, whose three rules
// they exist to make visible: no token material crosses the API boundary, a
// grant is the unit of consent (revoking it cascades to its sessions), and a
// listing only ever carries rows the caller is allowed to see (#561)

const GRANT_GRID = "1.2fr 1.3fr 1.6fr 150px 130px 96px 44px";
const SESSION_GRID = "1.1fr 1.2fr 1.4fr 140px 130px 150px 44px";

function stamp(iso: string): string {
  return iso.slice(0, 16).replace("T", " ");
}

function relative(iso: string, now: number): string {
  const secs = Math.round((now - Date.parse(iso)) / 1000);
  const ahead = secs < 0;
  const abs = Math.abs(secs);
  const unit =
    abs < 60
      ? `${abs}s`
      : abs < 3600
        ? `${Math.floor(abs / 60)}m`
        : abs < 86_400
          ? `${Math.floor(abs / 3600)}h`
          : `${Math.floor(abs / 86_400)}d`;
  return ahead ? `in ${unit}` : `${unit} ago`;
}

// the org's grants, sessions and the lookup tables that give them names. The
// users listing is best-effort: a member may not be allowed to read it, and
// the screens degrade to a short user id rather than failing the page
function useOrgOAuth(orgId?: string) {
  const servers = useQuery({
    queryKey: ["mcp-servers", orgId],
    queryFn: () => fetchMcpServers(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  const grants = useQuery({
    queryKey: ["mcp-grants", orgId],
    queryFn: () => fetchMcpGrants(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  const sessions = useQuery({
    queryKey: ["mcp-sessions", orgId],
    queryFn: () => fetchMcpSessions(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  const users = useQuery({
    queryKey: ["mcp-oauth-users", orgId],
    queryFn: () => fetchUsers(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  return { servers, grants, sessions, users };
}

function serverLabel(servers: McpServerRow[] | undefined, id: string): string {
  return servers?.find((s) => s.id === id)?.name ?? id.slice(0, 8);
}

function ownerLabel(users: UserRow[] | undefined, id: string): string {
  return users?.find((u) => u.id === id)?.email ?? id.slice(0, 8);
}

// stated on both screens because it is the one thing a reader cannot check for
// themselves: the tokens exist, they are just never serialised out of the
// control plane
function TokenNotice({ children }: { children: React.ReactNode }) {
  return (
    <p className="rounded-[10px] border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] px-3.5 py-2.5 text-xs leading-relaxed text-muted-foreground">
      {children}
    </p>
  );
}

// what a listing contains depends on who asked for it, so the screens say so
// instead of implying the table is the whole org
function ScopeNote({ noun }: { noun: string }) {
  return (
    <p className="text-xs text-[color:var(--text-subtle)]">
      Org admins see every {noun} in the org; members and viewers see — and may
      revoke — only their own.
    </p>
  );
}

function Scopes({ scopes }: { scopes: string[] }) {
  if (scopes.length === 0) {
    return <span className="text-xs text-[color:var(--text-subtle)]">no scopes</span>;
  }
  return (
    <div className="flex flex-wrap gap-1">
      {scopes.map((s) => (
        <Pill key={s} color="var(--text-secondary)" tint="var(--surface-subtle)">
          {s}
        </Pill>
      ))}
    </div>
  );
}

function ForbiddenNote({ noun, error }: { noun: string; error: Error }) {
  return (
    <p className="text-sm text-muted-foreground">
      You do not have access to this org&apos;s {noun}: {error.message}
    </p>
  );
}

function LoadingBody() {
  return (
    <PageBody>
      <Skeleton className="h-9 w-[320px] rounded-md" />
      <Skeleton className="h-[220px] rounded-lg" />
    </PageBody>
  );
}

function NoOrgNote({ noun }: { noun: string }) {
  return (
    <PageBody>
      <p className="text-sm text-muted-foreground">
        No org configured yet — pick or create one to view its {noun}.
      </p>
    </PageBody>
  );
}

// ---------------------------------------------------------------------------
// grants: the consent record a user signed for one MCP server

export function OAuthGrants() {
  const scope = useScope();
  const queryClient = useQueryClient();
  const { servers, grants, sessions, users } = useOrgOAuth(scope.orgId);
  const [confirming, setConfirming] = React.useState<McpOAuthGrantRow | null>(null);
  const now = Date.now();

  const revoke = useMutation({
    mutationFn: (id: string) => revokeMcpGrant(id),
    onSuccess: () => {
      setConfirming(null);
      // the cascade also killed sessions, so both listings are stale
      void queryClient.invalidateQueries({ queryKey: ["mcp-grants", scope.orgId] });
      void queryClient.invalidateQueries({ queryKey: ["mcp-sessions", scope.orgId] });
    },
  });

  // live sessions per grant: the number the revoke confirmation has to name,
  // because that is what the user is actually about to tear down
  const liveByGrant = React.useMemo(() => {
    const counts = new Map<string, number>();
    for (const s of sessions.data ?? []) {
      if (s.revoked_at || Date.parse(s.expires_at) <= now) continue;
      counts.set(s.grant_id, (counts.get(s.grant_id) ?? 0) + 1);
    }
    return counts;
  }, [now, sessions.data]);

  if (!scope.isLoading && !scope.orgId) return <NoOrgNote noun="OAuth grants" />;
  if (grants.isLoading || scope.isLoading) return <LoadingBody />;

  const rows = grants.data ?? [];
  const active = rows.filter((g) => g.active).length;
  const sessionsKnown = sessions.isSuccess;

  return (
    <PageBody>
      <TokenNotice>
        A grant records consent, not credentials. No access or refresh token is
        ever sent to this dashboard — the sealed tokens stay inside the control
        plane, and revoking consent here is what makes them unusable.
      </TokenNotice>

      {grants.isError ? (
        <ForbiddenNote noun="OAuth grants" error={grants.error as Error} />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <span className="text-sm text-muted-foreground">
              {rows.length} grants · {active} active
            </span>
            {revoke.isError && (
              <span className="text-xs text-destructive">
                {(revoke.error as Error).message}
              </span>
            )}
          </div>
          <ScopeNote noun="grant" />

          {rows.length === 0 ? (
            <EmptyState uxTarget="oauth-grants"
              icon={<Shield />}
              title="No consent granted yet"
              description="A grant appears the first time a user completes the OAuth flow for an MCP server. Nothing is created here — consent is only ever given by the user."
            />
          ) : (
            <ListTable>
              <ListHeader grid={GRANT_GRID}>
                <span>Server</span>
                <span>Owner</span>
                <span>Scopes</span>
                <span>Granted</span>
                <span>Sessions</span>
                <span>State</span>
                <span />
              </ListHeader>
              {rows.map((g) => {
                const live = liveByGrant.get(g.id) ?? 0;
                return (
                  <ListRow key={g.id} grid={GRANT_GRID}>
                    <span className="truncate font-mono text-xs font-semibold">
                      {serverLabel(servers.data, g.server_id)}
                    </span>
                    <span className="truncate text-xs text-[color:var(--text-secondary)]">
                      {ownerLabel(users.data, g.user_id)}
                    </span>
                    <Scopes scopes={g.scopes} />
                    <span
                      className="font-mono text-xs text-[color:var(--text-secondary)]"
                      title={g.granted_at}
                    >
                      {stamp(g.granted_at)}
                    </span>
                    <span className="text-xs text-[color:var(--text-secondary)]">
                      {sessionsKnown ? `${live} live` : "—"}
                    </span>
                    <Badge tone={g.active ? "success" : "neutral"} dot={g.active}>
                      {g.active ? "ACTIVE" : "REVOKED"}
                    </Badge>
                    <RowIconButton
                      danger
                      title={
                        g.active
                          ? "Revoke this consent and every session under it"
                          : "Already revoked"
                      }
                      aria-label={`Revoke consent for ${serverLabel(servers.data, g.server_id)}`}
                      disabled={!g.active || revoke.isPending}
                      onClick={() => setConfirming(g)}
                    >
                      <ShieldOff className="h-3.5 w-3.5" />
                    </RowIconButton>
                  </ListRow>
                );
              })}
            </ListTable>
          )}
        </>
      )}

      <Dialog
        open={confirming !== null}
        onOpenChange={(open) => !open && setConfirming(null)}
      >
        {confirming && (
          <>
            <DialogHeader>
              <DialogTitle>Revoke consent</DialogTitle>
              <DialogDescription>
                {/* the cascade is stated before the click, not discovered after
                    it: the server revokes the grant and its sessions in one
                    transaction */}
                Revoking this grant for{" "}
                <span className="font-mono">
                  {serverLabel(servers.data, confirming.server_id)}
                </span>{" "}
                also revokes{" "}
                {sessionsKnown
                  ? `${liveByGrant.get(confirming.id) ?? 0} live session${
                      (liveByGrant.get(confirming.id) ?? 0) === 1 ? "" : "s"
                    }`
                  : "every session"}{" "}
                under it, in the same transaction. The user must consent again
                before the server can be called on their behalf.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => setConfirming(null)}>
                Cancel
              </Button>
              <Button
                disabled={revoke.isPending}
                onClick={() => revoke.mutate(confirming.id)}
              >
                Revoke consent
              </Button>
            </DialogFooter>
          </>
        )}
      </Dialog>
    </PageBody>
  );
}

// ---------------------------------------------------------------------------
// sessions: the token metadata minted under a grant

type SessionState = "active" | "expired" | "revoked";

function sessionState(s: McpOAuthSessionRow, now: number): SessionState {
  if (s.revoked_at) return "revoked";
  return Date.parse(s.expires_at) <= now ? "expired" : "active";
}

export function AuthSessions() {
  const scope = useScope();
  const queryClient = useQueryClient();
  const { servers, grants, sessions, users } = useOrgOAuth(scope.orgId);
  // one clock for every relative timestamp, so rows do not drift apart
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);

  const revoke = useMutation({
    mutationFn: (id: string) => revokeMcpSession(id),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["mcp-sessions", scope.orgId] }),
  });

  const grantById = React.useMemo(() => {
    const map = new Map<string, McpOAuthGrantRow>();
    for (const g of grants.data ?? []) map.set(g.id, g);
    return map;
  }, [grants.data]);

  if (!scope.isLoading && !scope.orgId) return <NoOrgNote noun="auth sessions" />;
  if (sessions.isLoading || scope.isLoading) return <LoadingBody />;

  const rows = sessions.data ?? [];
  const live = rows.filter((s) => sessionState(s, now) === "active").length;

  return (
    <PageBody>
      <TokenNotice>
        Sessions are shown as metadata only. The access and refresh tokens are
        sealed with the deployment key and never leave the control plane, so
        this screen can tell you a session is renewable but never what it holds.
      </TokenNotice>

      {sessions.isError ? (
        <ForbiddenNote noun="auth sessions" error={sessions.error as Error} />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <span className="text-sm text-muted-foreground">
              {rows.length} sessions · {live} live
            </span>
            {revoke.isError && (
              <span className="text-xs text-destructive">
                {(revoke.error as Error).message}
              </span>
            )}
          </div>
          <ScopeNote noun="session" />

          {rows.length === 0 ? (
            <EmptyState uxTarget="auth-sessions"
              icon={<KeyRound />}
              title="No sessions yet"
              description="A session is minted when a user's consent is exchanged for a token. Grant an MCP server access from a client to see one here."
            />
          ) : (
            <ListTable>
              <ListHeader grid={SESSION_GRID}>
                <span>Server</span>
                <span>Owner</span>
                <span>Scopes</span>
                <span>Last used</span>
                <span>Expires</span>
                <span>State</span>
                <span />
              </ListHeader>
              {rows.map((s) => {
                const grant = grantById.get(s.grant_id);
                const state = sessionState(s, now);
                const server = grant
                  ? serverLabel(servers.data, grant.server_id)
                  : s.grant_id.slice(0, 8);
                return (
                  <ListRow key={s.id} grid={SESSION_GRID}>
                    <span className="truncate font-mono text-xs font-semibold">
                      {server}
                    </span>
                    <span className="truncate text-xs text-[color:var(--text-secondary)]">
                      {grant ? ownerLabel(users.data, grant.user_id) : "—"}
                    </span>
                    <Scopes scopes={s.scopes} />
                    <span className="text-xs text-[color:var(--text-secondary)]">
                      {s.last_used_at ? relative(s.last_used_at, now) : "never"}
                    </span>
                    <span
                      className="font-mono text-xs text-[color:var(--text-secondary)]"
                      title={s.expires_at}
                    >
                      {stamp(s.expires_at)}
                    </span>
                    <div className="flex flex-wrap items-center gap-1.5">
                      <Badge
                        tone={
                          state === "active"
                            ? "success"
                            : state === "expired"
                              ? "warning"
                              : "neutral"
                        }
                        dot={state === "active"}
                      >
                        {state.toUpperCase()}
                      </Badge>
                      {/* renewability is reported as a flag; the refresh token
                          itself is never part of the payload */}
                      {s.has_refresh_token && <Badge tone="info">RENEWABLE</Badge>}
                    </div>
                    <RowIconButton
                      danger
                      title={
                        state === "revoked"
                          ? "Already revoked"
                          : "Revoke this session; the consent behind it stands"
                      }
                      aria-label={`Revoke session on ${server}`}
                      disabled={state === "revoked" || revoke.isPending}
                      onClick={() => revoke.mutate(s.id)}
                    >
                      <ShieldOff className="h-3.5 w-3.5" />
                    </RowIconButton>
                  </ListRow>
                );
              })}
            </ListTable>
          )}
          <p className="text-xs text-[color:var(--text-subtle)]">
            Revoking a session leaves the consent in place — a new session can be
            minted without asking the user again. To withdraw consent itself,
            revoke the grant on OAuth Grants.
          </p>
        </>
      )}
    </PageBody>
  );
}
