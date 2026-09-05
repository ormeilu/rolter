import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { TFunction } from "i18next";
import { KeyRound, Loader2, Plus, ShieldCheck, Trash2, Users } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { CopyButton } from "@/components/CopyButton";
import { EditorSheet } from "@/components/EditorSheet";
import { LoadError } from "@/components/LoadError";
import { PageBody, Pill, RowIconButton } from "@/components/screen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  createSsoGroupMapping,
  createSsoProvider,
  deleteSsoGroupMapping,
  deleteSsoProvider,
  fetchAuthPolicy,
  fetchSsoGroupMappings,
  fetchSsoProviders,
  ROLES,
  ssoStartPath,
  updateAuthPolicy,
  type OrgAuthPolicy,
  type SsoGroupMappingRow,
  type SsoProviderRow,
} from "@/lib/api";
import { useScope } from "@/lib/scope";
import { errorDetail, useToast } from "@/lib/toast";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

const PROVIDERS_KEY = "sso-providers";
const POLICY_KEY = "org-auth-policy";
const MAPPINGS_KEY = "sso-group-mappings";

// the roles a group mapping may grant, mirroring `parse_role` in
// crates/rolter-control/src/sso.rs. deliberately not /api/v1/roles: that list
// carries every role the control plane knows about, and offering one the
// mapping endpoint refuses would build a form that can only fail on submit
const MAPPABLE_ROLES = ROLES;

// the label for a role the server sent us, falling back to the raw value so a
// newer control plane's role is shown rather than rendered as a missing key
function roleLabel(t: TFunction, role: string): string {
  return t(`shell.roles.${role}`, { defaultValue: role });
}

// a labelled line inside a provider card: mono value, optionally copyable
function Detail({
  label,
  value,
  copyLabel,
}: {
  label: string;
  value: string;
  copyLabel?: string;
}) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="w-[104px] flex-none text-[0.6875rem] uppercase tracking-[0.07em] text-[color:var(--text-subtle)]">
        {label}
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-[color:var(--text-secondary)]">
        {value}
      </span>
      {copyLabel && <CopyButton value={value} label={copyLabel} />}
    </div>
  );
}

/**
 * Which ways into the dashboard this org allows.
 *
 * Both flags are sent together because the control plane refuses the
 * *combination*, not the field: both off is an outage, and passwords off before
 * an enabled provider exists locks every non-superadmin out. Each is a 409 with
 * its own message, so the local guard below only covers the case an operator
 * can see for themselves.
 */
function SignInPolicyCard({
  orgId,
  policy,
}: {
  orgId: string;
  policy: OrgAuthPolicy;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [password, setPassword] = React.useState(policy.allow_password_login);
  const [sso, setSso] = React.useState(policy.allow_sso);

  // re-seed when the server's copy moves under us — another admin, or our own
  // save coming back
  React.useEffect(() => {
    setPassword(policy.allow_password_login);
    setSso(policy.allow_sso);
  }, [policy.allow_password_login, policy.allow_sso]);

  const save = useMutation({
    mutationFn: () =>
      updateAuthPolicy(orgId, {
        allow_password_login: password,
        allow_sso: sso,
      }),
    onSuccess: (next) => {
      queryClient.setQueryData([POLICY_KEY, orgId], next);
      void queryClient.invalidateQueries({ queryKey: [POLICY_KEY, orgId] });
      toast.push({
        tone: "success",
        title: t("toast.saved"),
        detail: t("toast.savedDetail", { what: t("errors.resources.signInPolicy") }),
      });
    },
    onError: (error) => {
      toast.push({
        tone: "error",
        title: t("toast.saveFailed", { what: t("errors.resources.signInPolicy") }),
        detail: errorDetail(error),
      });
    },
  });

  const dirty =
    password !== policy.allow_password_login || sso !== policy.allow_sso;
  const bothOff = !password && !sso;

  return (
    <section className="rounded-[10px] border border-[color:var(--border-subtle)] bg-[color:var(--surface-card)]">
      <header className="border-b border-[color:var(--border-subtle)] px-4 py-3">
        <h2 className="text-sm font-medium text-foreground">
          {t("pages.sso.policy.title")}
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("pages.sso.policy.subtitle")}
        </p>
      </header>
      <div className="flex flex-col gap-4 px-4 py-4">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <p className="text-sm font-medium text-foreground">
              {t("pages.sso.policy.passwordLabel")}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {t("pages.sso.policy.passwordHint")}
            </p>
          </div>
          <Switch
            checked={password}
            onCheckedChange={setPassword}
            aria-label={t("pages.sso.policy.passwordLabel")}
          />
        </div>
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <p className="text-sm font-medium text-foreground">
              {t("pages.sso.policy.ssoLabel")}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {t("pages.sso.policy.ssoHint")}
            </p>
          </div>
          <Switch
            checked={sso}
            onCheckedChange={setSso}
            aria-label={t("pages.sso.policy.ssoLabel")}
          />
        </div>
      </div>
      <footer className="flex flex-wrap items-center gap-3 border-t border-[color:var(--border-subtle)] px-4 py-3">
        {bothOff && (
          <p className="text-xs text-[color:var(--status-warning-text)]">
            {t("pages.sso.policy.bothOff")}
          </p>
        )}
        {/* the control plane's own words, never a gloss on them */}
        {save.isError && (
          <p role="alert" className="text-xs text-[color:var(--status-danger-text)]">
            {(save.error as Error).message}
          </p>
        )}
        <Button
          className="ml-auto"
          size="sm"
          disabled={!dirty || bothOff || save.isPending}
          onClick={() => save.mutate()}
        >
          {save.isPending && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
          {t("pages.sso.policy.save")}
        </Button>
      </footer>
    </section>
  );
}

/**
 * The IdP groups this provider turns into roles.
 *
 * Every mapping the dashboard creates is org-scoped: the create endpoint reads
 * an omitted scope as the provider's own org, and a mapping may never grant
 * outside it. Team- and project-scoped grants exist on the API and are not
 * offered here — see #1185.
 */
function GroupMappings({ provider }: { provider: SsoProviderRow }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const toast = useToast();
  const mappings = useQuery({
    queryKey: [MAPPINGS_KEY, provider.id],
    queryFn: () => fetchSsoGroupMappings(provider.id),
    retry: false,
  });

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: [MAPPINGS_KEY, provider.id] });

  const [group, setGroup] = React.useState("");
  const [role, setRole] = React.useState<string>(MAPPABLE_ROLES[0]);

  const create = useMutation({
    mutationFn: () =>
      createSsoGroupMapping(provider.id, {
        group_name: group.trim(),
        role,
      }),
    onSuccess: () => {
      toast.push({ tone: "success", title: t("toast.created", { what: group.trim() }) });
      setGroup("");
      invalidate();
    },
    onError: (error) => {
      toast.push({
        tone: "error",
        title: t("toast.saveFailed", { what: group.trim() }),
        detail: errorDetail(error),
      });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => deleteSsoGroupMapping(id),
    onSuccess: invalidate,
  });

  // a mapping is what puts people in a role, so removing one takes access away
  // from everyone in that group — named and confirmed first (#1179)
  const [removeTarget, setRemoveTarget] = React.useState<SsoGroupMappingRow | null>(
    null,
  );
  const startRemove = (mapping: SsoGroupMappingRow) => {
    remove.reset();
    setRemoveTarget(mapping);
  };

  const rows = mappings.data ?? [];

  return (
    <div className="border-t border-[color:var(--border-subtle)] px-4 py-3.5">
      <div className="flex items-center gap-1.5">
        <Users aria-hidden className="h-3.5 w-3.5 text-muted-foreground" />
        <h4 className="text-[0.6875rem] uppercase tracking-[0.07em] text-[color:var(--text-subtle)]">
          {t("pages.sso.mappings.title")}
        </h4>
      </div>

      {mappings.isLoading && <Skeleton className="mt-2.5 h-8 rounded-md" />}
      {mappings.isError && (
        <p className="mt-2.5 text-sm text-[color:var(--status-danger-text)]">
          {(mappings.error as Error).message}
        </p>
      )}
      {!mappings.isLoading && !mappings.isError && rows.length === 0 && (
        <p className="mt-2 text-sm text-muted-foreground">
          {provider.default_role
            ? t("pages.sso.mappings.emptyWithDefault", {
                role: roleLabel(t, provider.default_role),
              })
            : t("pages.sso.mappings.empty")}
        </p>
      )}

      {rows.length > 0 && (
        <ul className="mt-2 flex flex-col gap-1.5">
          {rows.map((mapping) => (
            <li
              key={mapping.id}
              className="flex items-center gap-2 rounded-md border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] px-2.5 py-1.5"
            >
              <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground">
                {mapping.group_name}
              </span>
              <Badge tone="neutral">{roleLabel(t, mapping.role)}</Badge>
              <RowIconButton
                danger
                title={t("pages.sso.mappings.remove")}
                aria-label={t("pages.sso.mappings.removeNamed", {
                  group: mapping.group_name,
                })}
                disabled={remove.isPending}
                onClick={() => startRemove(mapping)}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </RowIconButton>
            </li>
          ))}
        </ul>
      )}

      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        <Input
          className="h-8 max-w-[220px] flex-1"
          value={group}
          onChange={(e) => setGroup(e.target.value)}
          aria-label={t("pages.sso.mappings.groupLabel")}
          placeholder={t("pages.sso.mappings.groupPlaceholder")}
        />
        <Select
          className="h-8 w-[132px]"
          value={role}
          onChange={(e) => setRole(e.target.value)}
          aria-label={t("pages.sso.mappings.roleLabel")}
        >
          {MAPPABLE_ROLES.map((r) => (
            <option key={r} value={r}>
              {roleLabel(t, r)}
            </option>
          ))}
        </Select>
        <Button
          size="sm"
          variant="outline"
          disabled={!group.trim() || create.isPending}
          onClick={() => create.mutate()}
        >
          {create.isPending && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
          {t("pages.sso.mappings.add")}
        </Button>
      </div>
      {create.isError && (
        <p role="alert" className="mt-2 text-sm text-[color:var(--status-danger-text)]">
          {(create.error as Error).message}
        </p>
      )}

      <ConfirmDialog
        open={!!removeTarget}
        onOpenChange={(open) => !open && setRemoveTarget(null)}
        title={t("pages.sso.mappings.confirm.title", {
          group: removeTarget?.group_name,
        })}
        description={t("pages.sso.mappings.confirm.body", {
          role: removeTarget ? roleLabel(t, removeTarget.role) : "",
        })}
        confirmLabel={t("pages.sso.mappings.confirm.confirm")}
        pending={remove.isPending}
        error={remove.error}
        onConfirm={() => {
          if (!removeTarget) return;
          const what = removeTarget.group_name;
          remove.mutate(removeTarget.id, {
            onSuccess: () => {
              setRemoveTarget(null);
              toast.push({ tone: "success", title: t("toast.deleted", { what }) });
            },
            onError: (error) => {
              toast.push({
                tone: "error",
                title: t("toast.deleteFailed", { what }),
                detail: errorDetail(error),
              });
            },
          });
        }}
      />
    </div>
  );
}

function ProviderCard({
  provider,
  onDelete,
  deleting,
}: {
  provider: SsoProviderRow;
  onDelete: (provider: SsoProviderRow) => void;
  deleting: boolean;
}) {
  const { t } = useTranslation();
  // the login button's href, as the control plane builds it. absolute so it can
  // be pasted into a bookmark or an IdP's test console, not only followed here
  const startUrl = `${window.location.origin}${ssoStartPath(provider.slug)}`;

  return (
    <section className="rounded-[10px] border border-[color:var(--border-subtle)] bg-[color:var(--surface-card)]">
      <header className="flex items-start gap-3 px-4 py-3.5">
        <KeyRound aria-hidden className="mt-0.5 h-4 w-4 flex-none text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium text-foreground">{provider.name}</span>
            <Pill color="var(--text-secondary)" tint="var(--surface-subtle)">
              {provider.slug}
            </Pill>
            {provider.enabled ? (
              <Badge dot tone="success">
                {t("pages.sso.providers.enabled")}
              </Badge>
            ) : (
              <Badge tone="neutral">{t("pages.sso.providers.disabled")}</Badge>
            )}
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {provider.default_role
              ? t("pages.sso.providers.defaultRole", {
                  role: roleLabel(t, provider.default_role),
                })
              : t("pages.sso.providers.noDefaultRole")}
          </p>
        </div>
        <RowIconButton
          danger
          title={t("pages.sso.providers.delete")}
          aria-label={t("pages.sso.providers.deleteNamed", { name: provider.name })}
          disabled={deleting}
          onClick={() => onDelete(provider)}
        >
          {deleting ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Trash2 className="h-3.5 w-3.5" />
          )}
        </RowIconButton>
      </header>

      <div className="flex flex-col gap-1.5 border-t border-[color:var(--border-subtle)] px-4 py-3">
        <Detail label={t("pages.sso.providers.issuer")} value={provider.issuer} />
        <Detail
          label={t("pages.sso.providers.clientId")}
          value={provider.client_id}
        />
        <Detail
          label={t("pages.sso.providers.startUrl")}
          value={startUrl}
          copyLabel={t("pages.sso.providers.copyStartUrl")}
        />
        <Detail
          label={t("pages.sso.providers.groupClaim")}
          value={provider.group_claim}
        />
        <Detail
          label={t("pages.sso.providers.scopes")}
          value={provider.scopes.join(" ")}
        />
      </div>

      <GroupMappings provider={provider} />
    </section>
  );
}

interface Draft {
  name: string;
  slug: string;
  issuer: string;
  clientId: string;
  clientSecret: string;
  scopes: string;
  groupClaim: string;
  defaultRole: string;
}

const EMPTY_DRAFT: Draft = {
  name: "",
  slug: "",
  issuer: "",
  clientId: "",
  clientSecret: "",
  scopes: "",
  groupClaim: "",
  defaultRole: "",
};

// register a provider. the client secret is sealed with the KEK on the way in
// and `#[serde(skip_serializing)]` on the way out, so this form is the only
// place it is ever legible
function AddProviderSheet({
  open,
  onOpenChange,
  orgId,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  orgId: string;
  onCreated: () => void;
}) {
  const { t } = useTranslation();
  const toast = useToast();
  const [draft, setDraft] = React.useState<Draft>(EMPTY_DRAFT);

  React.useEffect(() => {
    if (open) setDraft(EMPTY_DRAFT);
  }, [open]);

  const create = useMutation({
    mutationFn: () =>
      createSsoProvider(orgId, {
        name: draft.name.trim(),
        slug: draft.slug.trim(),
        issuer: draft.issuer.trim(),
        client_id: draft.clientId.trim(),
        // an omitted secret is a public client; an empty string is not sent so
        // the server does not seal a blank
        client_secret: draft.clientSecret.trim() || undefined,
        scopes: draft.scopes.trim()
          ? draft.scopes.trim().split(/[\s,]+/).filter(Boolean)
          : undefined,
        group_claim: draft.groupClaim.trim() || undefined,
        default_role: draft.defaultRole || undefined,
      }),
    onSuccess: () => {
      // the sheet closes on success, so the outcome is announced somewhere
      // that outlives it (#1197)
      toast.push({ tone: "success", title: t("toast.created", { what: draft.name.trim() }) });
      onCreated();
      onOpenChange(false);
    },
    onError: (error) => {
      toast.push({
        tone: "error",
        title: t("toast.saveFailed", { what: draft.name.trim() }),
        detail: errorDetail(error),
      });
    },
  });

  const set = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));
  const dirty = Object.values(draft).some((v) => v !== "");
  const canSave =
    !!draft.name.trim() &&
    !!draft.slug.trim() &&
    !!draft.issuer.trim() &&
    !!draft.clientId.trim();

  return (
    <EditorSheet
      open={open}
      onOpenChange={onOpenChange}
      title={t("pages.sso.create.title")}
      subtitle={t("pages.sso.create.subtitle")}
      dirty={dirty}
      errorMessage={create.isError ? (create.error as Error).message : undefined}
      saveLabel={t("pages.sso.create.save")}
      canSave={canSave}
      saving={create.isPending}
      onSave={() => create.mutate()}
    >
      <Field
        label={t("pages.sso.create.name")}
        hint={t("pages.sso.create.nameHint")}
      >
        <Input
          value={draft.name}
          onChange={(e) => set({ name: e.target.value })}
          placeholder={t("pages.sso.create.namePlaceholder")}
        />
      </Field>
      <Field
        label={t("pages.sso.create.slug")}
        hint={t("pages.sso.create.slugHint")}
      >
        <Input
          value={draft.slug}
          onChange={(e) => set({ slug: e.target.value })}
          placeholder={t("pages.sso.create.slugPlaceholder")}
        />
      </Field>
      <Field
        label={t("pages.sso.create.issuer")}
        hint={t("pages.sso.create.issuerHint")}
      >
        <Input
          value={draft.issuer}
          onChange={(e) => set({ issuer: e.target.value })}
          placeholder={t("pages.sso.create.issuerPlaceholder")}
        />
      </Field>
      <Field
        label={t("pages.sso.create.clientId")}
        hint={t("pages.sso.create.clientIdHint")}
      >
        <Input
          value={draft.clientId}
          onChange={(e) => set({ clientId: e.target.value })}
          placeholder={t("pages.sso.create.clientIdPlaceholder")}
        />
      </Field>
      <Field
        label={t("pages.sso.create.clientSecret")}
        hint={t("pages.sso.create.clientSecretHint")}
      >
        <Input
          type="password"
          value={draft.clientSecret}
          onChange={(e) => set({ clientSecret: e.target.value })}
        />
      </Field>
      <p className="text-sm font-medium text-[color:var(--status-warning-text)]">
        {t("pages.sso.create.secretWriteOnly")}
      </p>
      <Field
        label={t("pages.sso.create.scopes")}
        hint={t("pages.sso.create.scopesHint")}
      >
        <Input
          value={draft.scopes}
          onChange={(e) => set({ scopes: e.target.value })}
          placeholder={t("pages.sso.create.scopesPlaceholder")}
        />
      </Field>
      <Field
        label={t("pages.sso.create.groupClaim")}
        hint={t("pages.sso.create.groupClaimHint")}
      >
        <Input
          value={draft.groupClaim}
          onChange={(e) => set({ groupClaim: e.target.value })}
          placeholder={t("pages.sso.create.groupClaimPlaceholder")}
        />
      </Field>
      <Field
        label={t("pages.sso.create.defaultRole")}
        hint={t("pages.sso.create.defaultRoleHint")}
      >
        <Select
          value={draft.defaultRole}
          onChange={(e) => set({ defaultRole: e.target.value })}
        >
          <option value="">{t("pages.sso.create.defaultRoleNone")}</option>
          {MAPPABLE_ROLES.map((r) => (
            <option key={r} value={r}>
              {roleLabel(t, r)}
            </option>
          ))}
        </Select>
      </Field>
    </EditorSheet>
  );
}

/**
 * Governance › Single sign-on (#1185).
 *
 * The control plane has carried OIDC SSO and a per-org sign-in policy since
 * #240, but nothing in the dashboard could register a provider: the login
 * screen rendered "Continue with …" buttons for providers only a direct API
 * call could create. This is that screen.
 */
export default function SingleSignOn() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const toast = useToast();
  const scope = useScope();
  const orgId = scope.orgId;

  const providers = useQuery({
    queryKey: [PROVIDERS_KEY, orgId],
    queryFn: () => fetchSsoProviders(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  const policy = useQuery({
    queryKey: [POLICY_KEY, orgId],
    queryFn: () => fetchAuthPolicy(orgId as string),
    enabled: !!orgId,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // the provider list is what the user is actually waiting on here
  useScreenReady(!providers.isLoading);
  useErrorState(!!providers.error, "sso");

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: [PROVIDERS_KEY, orgId] });

  const remove = useMutation({
    mutationFn: (id: string) => deleteSsoProvider(id),
    onSuccess: invalidate,
  });

  const [addOpen, setAddOpen] = React.useState(false);
  const [deleteTarget, setDeleteTarget] = React.useState<SsoProviderRow | null>(
    null,
  );
  const startDelete = (provider: SsoProviderRow) => {
    remove.reset();
    setDeleteTarget(provider);
  };

  if (scope.isLoading || (!!orgId && (providers.isLoading || policy.isLoading))) {
    return (
      <PageBody>
        <Skeleton className="h-[196px] rounded-[10px]" />
        <Skeleton className="h-9 w-[280px] rounded-md" />
        <Skeleton className="h-[240px] rounded-[10px]" />
      </PageBody>
    );
  }

  const rows = providers.data ?? [];
  // no org means nothing to hang a provider on, and an unreadable list means
  // this principal may not manage them either
  const canManage = !!orgId && !providers.isError;

  return (
    <PageBody>
      {policy.isError && (
        <LoadError
          error={policy.error}
          resource={t("errors.resources.signInPolicy")}
          onRetry={() => policy.refetch()}
        />
      )}
      {policy.data && orgId && (
        <SignInPolicyCard orgId={orgId} policy={policy.data} />
      )}

      <div className="flex flex-wrap items-center gap-3">
        <h2 className="text-sm font-medium text-foreground">
          {t("pages.sso.providers.title")}
        </h2>
        <span className="text-sm text-muted-foreground">
          {t("pages.sso.providers.count", { count: rows.length })}
        </span>
        <Button
          className="ml-auto"
          disabled={!canManage}
          onClick={() => setAddOpen(true)}
        >
          <Plus className="h-4 w-4" aria-hidden />
          {t("pages.sso.providers.add")}
        </Button>
      </div>

      {providers.isError && (
        <LoadError
          error={providers.error}
          resource={t("errors.resources.ssoProviders")}
          onRetry={() => providers.refetch()}
        />
      )}

      {!providers.isError &&
        (rows.length === 0 ? (
          <EmptyState
            uxTarget="sso-providers"
            icon={<ShieldCheck />}
            title={t("pages.sso.empty.title")}
            description={t("pages.sso.empty.body")}
            actions={
              <Button disabled={!canManage} onClick={() => setAddOpen(true)}>
                <Plus className="h-4 w-4" aria-hidden />
                {t("pages.sso.providers.add")}
              </Button>
            }
          />
        ) : (
          <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(min(420px,100%),1fr))]">
            {rows.map((provider) => (
              <ProviderCard
                key={provider.id}
                provider={provider}
                deleting={remove.isPending && remove.variables === provider.id}
                onDelete={startDelete}
              />
            ))}
          </div>
        ))}

      {orgId && (
        <AddProviderSheet
          open={addOpen}
          onOpenChange={setAddOpen}
          orgId={orgId}
          onCreated={invalidate}
        />
      )}

      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title={t("pages.sso.confirm.title", { name: deleteTarget?.name })}
        description={t("pages.sso.confirm.body")}
        confirmLabel={t("pages.sso.confirm.confirm")}
        pending={remove.isPending}
        error={remove.error}
        onConfirm={() => {
          if (!deleteTarget) return;
          const what = deleteTarget.name;
          remove.mutate(deleteTarget.id, {
            onSuccess: () => {
              setDeleteTarget(null);
              toast.push({ tone: "success", title: t("toast.deleted", { what }) });
            },
            onError: (error) => {
              toast.push({
                tone: "error",
                title: t("toast.deleteFailed", { what }),
                detail: errorDetail(error),
              });
            },
          });
        }}
      />
    </PageBody>
  );
}
