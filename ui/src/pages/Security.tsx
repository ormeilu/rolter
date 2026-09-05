import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { LoadError } from "@/components/LoadError";
import { PanelSkeleton } from "@/components/LoadingState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  fetchSecuritySettings,
  updateSecuritySettings,
  type SecuritySettingsDto,
} from "@/lib/api";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

interface FormState {
  authEnabled: boolean;
  credentialRef: string;
  managedSecret: string;
  enforceVk: boolean;
  allowedOrigins: string;
  allowedHeaders: string;
  requiredHeaders: string;
  bypassRoutes: string;
}

const splitList = (value: string) =>
  value
    .split(",")
    .map((v) => v.trim())
    .filter(Boolean);

const parseRequiredHeaders = (value: string): Record<string, string> => {
  const headers: Record<string, string> = {};
  for (const pair of splitList(value)) {
    const idx = pair.indexOf(":");
    if (idx > 0) {
      headers[pair.slice(0, idx).trim()] = pair.slice(idx + 1).trim();
    }
  }
  return headers;
};

const fromDto = (dto: SecuritySettingsDto): FormState => ({
  authEnabled: dto.dashboard_auth_enabled,
  credentialRef: dto.dashboard_credential_ref ?? "",
  managedSecret: "",
  enforceVk: dto.virtual_key_required,
  allowedOrigins: dto.allowed_origins.join(", "),
  allowedHeaders: dto.allowed_headers.join(", "),
  requiredHeaders: Object.entries(
    (dto.required_headers ?? {}) as Record<string, string>,
  )
    .map(([k, v]) => `${k}: ${v}`)
    .join(", "),
  bypassRoutes: dto.auth_bypass_routes.join(", "),
});

// global gateway security policy, persisted via /api/v1/security-settings
// (superadmin only). dashboard secret is write-only: the server seals it and
// reports only whether one is configured.
export default function Security() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const settings = useQuery({
    queryKey: ["security-settings"],
    queryFn: fetchSecuritySettings,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `settings` is the query the user is actually waiting on for this screen
  useScreenReady(!settings.isLoading);
  useErrorState(!!settings.error, "security");

  const [form, setForm] = React.useState<FormState | null>(null);
  const [saved, setSaved] = React.useState(false);
  React.useEffect(() => {
    if (settings.data && form === null) {
      setForm(fromDto(settings.data));
    }
  }, [settings.data, form]);

  const save = useMutation({
    mutationFn: (f: FormState) =>
      updateSecuritySettings({
        virtual_key_required: f.enforceVk,
        allowed_origins: splitList(f.allowedOrigins),
        allowed_headers: splitList(f.allowedHeaders),
        required_headers: parseRequiredHeaders(f.requiredHeaders),
        auth_bypass_routes: splitList(f.bypassRoutes),
        dashboard_auth_enabled: f.authEnabled,
        dashboard_credential_ref: f.credentialRef.trim() || null,
        ...(f.managedSecret.trim()
          ? { managed_dashboard_secret: f.managedSecret }
          : {}),
      }),
    onSuccess: (dto) => {
      queryClient.setQueryData(["security-settings"], dto);
      setForm(fromDto(dto));
      setSaved(true);
    },
  });

  if (settings.isLoading) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        <PanelSkeleton panels={3} height={148} />
      </div>
    );
  }
  if (settings.isError) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        <LoadError
          error={settings.error}
          resource={t("errors.resources.securitySettings")}
          onRetry={() => void settings.refetch()}
        />
      </div>
    );
  }
  if (!form) return null;

  const set = (patch: Partial<FormState>) => {
    setForm((f) => (f ? { ...f, ...patch } : f));
    setSaved(false);
  };
  const disabledAuth = !form.authEnabled;
  const secretConfigured = settings.data?.dashboard_secret_configured ?? false;

  return (
    <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div className="flex items-start gap-4">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span id="security-auth-label" className="text-sm font-medium">Password protect the dashboard</span>
              <Badge tone="info">BETA</Badge>
            </div>
            <p className="mt-1 text-sm text-muted-foreground">
              Require a credential to open the dashboard. Provide either an external
              secret-manager reference or a managed secret stored encrypted.
            </p>
          </div>
          <Switch
            checked={form.authEnabled}
            aria-labelledby="security-auth-label"
            onCheckedChange={(v) => set({ authEnabled: v })}
          />
        </div>
        {/* a disabled fieldset rather than a dimmed div: the inputs inside
            already carry `disabled`, and fading a live div drags its labels and
            hints below 4.5:1 while telling assistive tech nothing (#1181) */}
        <fieldset
          className="flex min-w-0 flex-col gap-1.5"
          disabled={disabledAuth}
          style={{ opacity: disabledAuth ? 0.55 : 1 }}
        >
          <label htmlFor="security-credential-ref" className="text-xs font-medium text-[color:var(--text-secondary)]">
            Credential reference
          </label>
          <Input
            id="security-credential-ref"
            value={form.credentialRef}
            disabled={disabledAuth}
            placeholder="vault://secrets/rolter-dashboard"
            onChange={(e) => set({ credentialRef: e.target.value })}
          />
        </fieldset>
        <fieldset
          className="flex min-w-0 flex-col gap-1.5"
          disabled={disabledAuth}
          style={{ opacity: disabledAuth ? 0.55 : 1 }}
        >
          <label htmlFor="security-managed-secret" className="text-xs font-medium text-[color:var(--text-secondary)]">
            Managed secret
          </label>
          <Input
            id="security-managed-secret"
            type="password"
            value={form.managedSecret}
            disabled={disabledAuth}
            placeholder={
              secretConfigured ? "configured — enter to replace" : "Enter a secret to store"
            }
            onChange={(e) => set({ managedSecret: e.target.value })}
          />
          <span className="text-[0.6875rem] text-[color:var(--text-subtle)]">
            Write-only: the secret is encrypted server-side and never shown again.
          </span>
        </fieldset>
      </section>

      <ToggleCard
        title="Enforce Virtual Keys on Inference"
        desc="Require a virtual key for all inference requests."
        checked={form.enforceVk}
        onChange={(v) => set({ enforceVk: v })}
      />
      <TextCard
        title="Allowed Origins"
        desc="Comma-separated list of exact http(s) origins allowed for CORS and WebSocket connections. Wildcards are rejected — list each origin explicitly."
        value={form.allowedOrigins}
        placeholder="https://app.example.com, https://console.example.com"
        onChange={(v) => set({ allowedOrigins: v })}
      />
      <TextCard
        title="Allowed Headers"
        desc="Comma-separated list of allowed headers for CORS."
        value={form.allowedHeaders}
        placeholder="X-Stainless-Timeout"
        onChange={(v) => set({ allowedHeaders: v })}
      />
      <TextCard
        title="Required Headers"
        desc="Comma-separated name: value pairs that must be present on every request. Requests missing any of them are rejected."
        value={form.requiredHeaders}
        placeholder="X-Tenant-ID: acme, X-Custom-Header: value"
        onChange={(v) => set({ requiredHeaders: v })}
      />
      <TextCard
        title="Auth Bypass Routes"
        desc="Comma-separated exact /v1 paths that skip the auth middleware. System routes like /health and the login endpoints are always open."
        value={form.bypassRoutes}
        placeholder="/v1/models, /v1/ping"
        onChange={(v) => set({ bypassRoutes: v })}
      />

      <div className="sticky bottom-0 flex items-center justify-end gap-3 border-t border-[color:var(--border-subtle)] bg-background py-3">
        {save.isError && (
          <span className="text-xs text-[color:var(--status-danger-text)]">
            {(save.error as Error).message}
          </span>
        )}
        {saved && (
          <span className="text-xs text-[color:var(--status-success-text)]">
            Security settings updated.
          </span>
        )}
        <Button disabled={save.isPending} onClick={() => save.mutate(form)}>
          {save.isPending ? "Saving…" : "Save Changes"}
        </Button>
      </div>
    </div>
  );
}

function ToggleCard({
  title,
  desc,
  checked,
  onChange,
}: {
  title: string;
  desc: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <section className="flex items-start gap-4 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
      <div className="min-w-0 flex-1">
        <span className="text-sm font-medium">{title}</span>
        <p className="mt-1 text-sm text-muted-foreground">{desc}</p>
      </div>
      <Switch checked={checked} onCheckedChange={onChange} aria-label={title} />
    </section>
  );
}

function TextCard({
  title,
  desc,
  value,
  placeholder,
  onChange,
}: {
  title: string;
  desc: string;
  value: string;
  placeholder: string;
  onChange: (v: string) => void;
}) {
  return (
    <section className="flex flex-col gap-2.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
      <div>
        <span className="text-sm font-medium">{title}</span>
        <p className="mt-1 text-sm text-muted-foreground">{desc}</p>
      </div>
      <Textarea
        className="min-h-[76px] font-mono text-xs"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
    </section>
  );
}
