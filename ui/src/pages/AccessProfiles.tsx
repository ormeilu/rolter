import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Shield, Trash2, Users } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { PageBody, Pill } from "@/components/screen";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  createAccessProfile,
  deleteAccessProfile,
  fetchAccessProfileAssignments,
  fetchAccessProfiles,
  type AccessProfileRow,
} from "@/lib/api";
import { useScope } from "@/lib/scope";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

/// One profile card, with the assignments it reaches.
///
/// Assignments are fetched per profile rather than in one call because the API
/// scopes them that way; a deployment has few profiles, so this stays a handful
/// of requests rather than a fan-out worth batching.
function ProfileCard({
  profile,
  onDelete,
  deleting,
}: {
  profile: AccessProfileRow;
  onDelete: (profile: AccessProfileRow) => void;
  deleting: boolean;
}) {
  const assignments = useQuery({
    queryKey: ["access-profile-assignments", profile.id],
    queryFn: () => fetchAccessProfileAssignments(profile.id),
  });

  const users = assignments.data?.filter((a) => a.user_id).length ?? 0;
  const teams = assignments.data?.filter((a) => a.team_id).length ?? 0;

  return (
    <div className="rounded-lg border p-4">
      <div className="flex items-start gap-3">
        <Shield className="mt-0.5 size-4 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-medium">{profile.name}</span>
            <Pill color="var(--text-secondary)" tint="var(--surface-subtle)">
              {profile.slug}
            </Pill>
          </div>
          {profile.description && (
            <p className="mt-1 text-sm text-muted-foreground">
              {profile.description}
            </p>
          )}
          <div className="mt-2 flex items-center gap-1.5 text-sm text-muted-foreground">
            <Users className="size-3.5" />
            {assignments.isLoading ? (
              <span>Loading assignments…</span>
            ) : (
              // a profile that reaches nobody grants nothing, which is worth
              // saying plainly rather than showing as "0 users, 0 teams"
              <span>
                {users === 0 && teams === 0
                  ? "Not assigned to anyone yet"
                  : `${users} user${users === 1 ? "" : "s"} · ${teams} team${teams === 1 ? "" : "s"}`}
              </span>
            )}
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          title="Delete profile"
          aria-label={`Delete ${profile.name}`}
          disabled={deleting}
          onClick={() => onDelete(profile)}
        >
          {deleting ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <Trash2 className="size-4" />
          )}
        </Button>
      </div>
    </div>
  );
}

export default function AccessProfiles() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const scope = useScope();
  const orgId = scope.orgId as string | undefined;

  const profiles = useQuery({
    queryKey: ["access-profiles", orgId],
    queryFn: () => fetchAccessProfiles(orgId as string),
    enabled: !!orgId,
  });

  useScreenReady(!profiles.isLoading);
  useErrorState(!!profiles.error, "access-profiles");

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["access-profiles", orgId] });

  const create = useMutation({
    mutationFn: (input: { name: string; slug?: string; description?: string }) =>
      createAccessProfile(orgId as string, input),
    onSuccess: () => {
      setOpen(false);
      invalidate();
    },
  });
  const remove = useMutation({
    mutationFn: deleteAccessProfile,
    onSuccess: invalidate,
  });

  const [open, setOpen] = React.useState(false);
  // a profile reaches users and teams, so deleting it changes what a group of
  // people can do — confirmed by name first (#1179)
  const [deleteTarget, setDeleteTarget] = React.useState<AccessProfileRow | null>(
    null,
  );
  const startDelete = (profile: AccessProfileRow) => {
    remove.reset();
    setDeleteTarget(profile);
  };
  const [name, setName] = React.useState("");
  const [slug, setSlug] = React.useState("");
  const [description, setDescription] = React.useState("");

  const startCreate = () => {
    setName("");
    setSlug("");
    setDescription("");
    setOpen(true);
  };

  return (
    <PageBody>
      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {profiles.data?.length ?? 0} profiles · reusable permission bundles
        </span>
        <Button className="ml-auto" disabled={!orgId} onClick={startCreate}>
          + Add profile
        </Button>
      </div>

      {profiles.isLoading && (
        <p className="text-sm text-muted-foreground">Loading…</p>
      )}
      {profiles.isError && (
        <p className="text-sm text-muted-foreground">
          Access profiles need admin access: {(profiles.error as Error).message}
        </p>
      )}
      {profiles.data && profiles.data.length === 0 && (
        <EmptyState
          icon={<Shield className="size-5" />}
          title="No access profiles yet"
          description="A profile bundles custom roles and a model policy, then hands the bundle to a user or a whole team."
          uxTarget="access-profiles"
        />
      )}

      <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(380px,1fr))]">
        {(profiles.data ?? []).map((profile) => (
          <ProfileCard
            key={profile.id}
            profile={profile}
            deleting={remove.isPending && remove.variables === profile.id}
            onDelete={startDelete}
          />
        ))}
      </div>

      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(o) => !o && setDeleteTarget(null)}
        title={t("pages.accessProfiles.confirm.title", { name: deleteTarget?.name })}
        description={t("pages.accessProfiles.confirm.body")}
        confirmLabel={t("pages.accessProfiles.confirm.confirm")}
        pending={remove.isPending}
        error={remove.error}
        onConfirm={() =>
          deleteTarget &&
          remove.mutate(deleteTarget.id, { onSuccess: () => setDeleteTarget(null) })
        }
      />

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogHeader>
          <DialogTitle>Add access profile</DialogTitle>
          <DialogDescription>
            A profile is a reusable bundle. Assign it to a user or a team, and
            everyone it reaches picks up the roles and policy it carries.
          </DialogDescription>
        </DialogHeader>
        <Field label="Name">
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Support engineers"
          />
        </Field>
        <Field label="Slug" hint="Derived from the name when left blank">
          <Input
            value={slug}
            onChange={(e) => setSlug(e.target.value)}
            placeholder="support-engineers"
          />
        </Field>
        <Field label="Description">
          <Input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Read-only access to logs and analytics"
          />
        </Field>
        {create.isError && (
          <p className="text-sm text-destructive">
            {(create.error as Error).message}
          </p>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            disabled={!name.trim() || create.isPending}
            onClick={() =>
              create.mutate({
                name: name.trim(),
                slug: slug.trim() || undefined,
                description: description.trim() || undefined,
              })
            }
          >
            {create.isPending ? "Creating…" : "Create profile"}
          </Button>
        </DialogFooter>
      </Dialog>
    </PageBody>
  );
}
