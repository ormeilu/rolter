import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { Building } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { EditorSheet } from "@/components/EditorSheet";
import { LoadError } from "@/components/LoadError";
import { PageBody } from "@/components/screen";
import { Button } from "@/components/ui/button";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { createTeam, fetchBudgets, fetchMemberships, fetchTeams } from "@/lib/api";
import { useScope } from "@/lib/scope";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

// teams from the design prototype: card per team with member count, the
// team-scoped budget (when one exists), and the team admin
export default function Teams() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const scope = useScope();

  const teams = useQuery({
    queryKey: ["teams", scope.orgId],
    queryFn: () => fetchTeams(scope.orgId as string),
    enabled: !!scope.orgId,
  });


  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;

  // `teams` is the query the user is actually waiting on for this screen

  useScreenReady(!teams.isLoading);

  useErrorState(!!teams.error, "teams");
  const memberships = useQuery({
    queryKey: ["memberships", scope.orgId],
    queryFn: () => fetchMemberships(scope.orgId as string),
    enabled: !!scope.orgId,
    retry: false,
  });
  const budgetQueries = useQueries({
    queries: (teams.data ?? []).map((team) => ({
      queryKey: ["budgets", "team", team.id],
      queryFn: () => fetchBudgets("team", team.id),
      retry: false,
    })),
  });

  const [addOpen, setAddOpen] = React.useState(false);
  const [name, setName] = React.useState("");

  const create = useMutation({
    mutationFn: () => createTeam(scope.orgId as string, { name }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["teams", scope.orgId] });
      setAddOpen(false);
      setName("");
    },
  });

  return (
    <PageBody>
      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {teams.data?.length ?? 0} teams · group users, share budgets and access
        </span>
        <Button className="ml-auto" onClick={() => setAddOpen(true)} disabled={!scope.orgId}>
          + New team
        </Button>
      </div>

      {teams.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {teams.error && (
        <LoadError
          error={teams.error}
          resource={t("errors.resources.teams")}
          onRetry={() => void teams.refetch()}
        />
      )}
      <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(320px,1fr))]">
        {(teams.data ?? []).map((team, i) => {
          const budget = budgetQueries[i]?.data?.[0];
          const members =
            memberships.data?.filter((m) => m.team_id === team.id) ?? [];
          const admin = members.find((m) => m.role === "admin");
          return (
            <div
              key={team.id}
              className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-default)] bg-card p-4"
            >
              <div className="flex items-center gap-2.5">
                <span className="flex h-[34px] w-[34px] flex-none items-center justify-center rounded-lg border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] text-[color:var(--text-secondary)]">
                  <Building className="h-4 w-4" />
                </span>
                <div className="min-w-0">
                  <div className="font-mono text-sm font-semibold">{team.name}</div>
                  <div className="truncate text-xs text-muted-foreground">
                    created {team.created_at?.slice(0, 10)}
                  </div>
                </div>
              </div>
              <div className="grid grid-cols-2 gap-2.5">
                <div>
                  <div className="mb-0.5 text-[0.6875rem] uppercase tracking-[0.05em] text-[color:var(--text-subtle)]">
                    Members
                  </div>
                  <div className="font-mono text-sm text-[color:var(--text-secondary)]">
                    {memberships.isError ? "—" : members.length}
                  </div>
                </div>
                <div>
                  <div className="mb-0.5 text-[0.6875rem] uppercase tracking-[0.05em] text-[color:var(--text-subtle)]">
                    Budget
                  </div>
                  <div className="font-mono text-sm text-[color:var(--text-secondary)]">
                    {budget ? `$${budget.limit_usd} / ${budget.period}` : "—"}
                  </div>
                </div>
              </div>
              {admin && (
                <div className="flex items-center gap-2 border-t border-[color:var(--border-subtle)] pt-3">
                  <span className="text-xs text-[color:var(--text-subtle)]">admin</span>
                  <span className="ml-auto truncate font-mono text-xs text-[color:var(--text-secondary)]">
                    {admin.user_id}
                  </span>
                </div>
              )}
            </div>
          );
        })}
      </div>

      <EditorSheet
        open={addOpen}
        onOpenChange={(open) => {
          setAddOpen(open);
          if (!open) setName("");
        }}
        title="New team"
        subtitle="Group users, share budgets and access."
        dirty={name.trim() !== ""}
        errorMessage={create.isError ? (create.error as Error).message : undefined}
        saveLabel="Create"
        canSave={!!name.trim()}
        saving={create.isPending}
        onSave={() => create.mutate()}
      >
        <Field label="Team name">
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="platform" />
        </Field>
      </EditorSheet>
    </PageBody>
  );
}
