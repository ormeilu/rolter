import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, Plus, Puzzle, Webhook } from "lucide-react";
import * as React from "react";

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
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  createPlugin,
  deletePlugin,
  fetchPlugins,
  updatePlugin,
  type PluginInstanceInput,
  type PluginInstanceRow,
} from "@/lib/api";
import { useScope } from "@/lib/scope";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

type Stage = PluginInstanceRow["stage"];

const STAGES: Array<{ key: Stage; label: string; description: string }> = [
  { key: "pre_route", label: "Pre-route", description: "Inspect or enrich the request before Rolter selects a route." },
  { key: "pre_upstream", label: "Pre-upstream", description: "Transform the routed request immediately before provider delivery." },
  { key: "post_response", label: "Post-response", description: "Inspect or transform the provider response before client delivery." },
];

const asInput = (plugin: PluginInstanceRow): PluginInstanceInput => ({
  project_id: plugin.project_id,
  name: plugin.name,
  slug: plugin.slug,
  description: plugin.description,
  kind: plugin.kind,
  stage: plugin.stage,
  enabled: plugin.enabled,
  position: plugin.position,
  failure_mode: plugin.failure_mode,
  endpoint: plugin.endpoint,
  secret_env: plugin.secret_env,
  config: plugin.config,
});

export default function Plugins() {
  const scope = useScope();
  const client = useQueryClient();
  const query = useQuery({
    queryKey: ["plugins", scope.orgId],
    queryFn: () => fetchPlugins(scope.orgId as string),
    enabled: Boolean(scope.orgId),
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `query` is the query the user is actually waiting on for this screen
  useScreenReady(!query.isLoading);
  useErrorState(!!query.error, "plugins");
  const [editing, setEditing] = React.useState<PluginInstanceRow | null | undefined>();
  const invalidate = () => client.invalidateQueries({ queryKey: ["plugins", scope.orgId] });
  const toggle = useMutation({
    mutationFn: (plugin: PluginInstanceRow) => updatePlugin(plugin.id, { ...asInput(plugin), enabled: !plugin.enabled }),
    onSuccess: invalidate,
  });
  const remove = useMutation({ mutationFn: deletePlugin, onSuccess: invalidate });

  if (scope.isLoading) {
    return <PluginLoading />;
  }
  if (scope.error || !scope.orgId) {
    return <p className="p-[22px] text-sm text-muted-foreground">{scope.error ?? "Choose an organization to manage plugins."}</p>;
  }

  const plugins = query.data ?? [];
  return (
    <div className="mx-auto flex max-w-[1180px] flex-col gap-5 p-[22px]">
      <header className="flex flex-col gap-3 border-b border-[color:var(--border-subtle)] pb-5 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <p className="font-mono text-[0.6875rem] uppercase tracking-[0.16em] text-[color:var(--status-info)]">Pipeline workbench</p>
            <Badge tone="warning">runtime hook pending</Badge>
          </div>
          <h1 className="mt-1 text-xl font-semibold tracking-tight">Installed plugin configuration</h1>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
            Arrange desired request and response middleware. Configuration is persisted now; gateway dispatch lands with the plugin runtime.
          </p>
        </div>
        <Button onClick={() => setEditing(null)}><Plus className="h-4 w-4" aria-hidden /> Install plugin</Button>
      </header>

      {query.isLoading ? <PluginLoading /> : query.isError ? (
        <section className="rounded-[10px] border border-[color:var(--status-danger)]/30 bg-[color:var(--status-danger)]/5 p-5">
          <p className="text-sm font-medium text-[color:var(--status-danger)]">Plugin registry unavailable</p>
          <p className="mt-1 text-sm text-muted-foreground">{(query.error as Error).message}</p>
          <Button className="mt-4" variant="outline" onClick={() => void query.refetch()}>Try again</Button>
        </section>
      ) : plugins.length === 0 ? (
        <EmptyState uxTarget="plugin-list"
          icon={<Puzzle />}
          title="No plugin configurations"
          description="Install a webhook configuration, choose its pipeline stage, and keep it paused until the gateway plugin runtime is available."
          actions={<Button onClick={() => setEditing(null)}>Install first plugin</Button>}
        />
      ) : (
        <div className="space-y-3">
          {STAGES.map((stage, index) => (
            <React.Fragment key={stage.key}>
              <StageLane
                stage={stage}
                plugins={plugins.filter((plugin) => plugin.stage === stage.key)}
                projectNames={Object.fromEntries(scope.projects.map((project) => [project.id, project.name]))}
                pending={toggle.isPending || remove.isPending}
                onToggle={(plugin) => toggle.mutate(plugin)}
                onEdit={setEditing}
                onDelete={(plugin) => window.confirm(`Delete ${plugin.name}? This removes its saved configuration.`) && remove.mutate(plugin.id)}
              />
              {index < STAGES.length - 1 && <ArrowDown className="mx-auto h-4 w-4 text-[color:var(--text-subtle)]" aria-hidden />}
            </React.Fragment>
          ))}
        </div>
      )}

      <PluginDialog
        key={editing?.id ?? (editing === null ? "new" : "closed")}
        open={editing !== undefined}
        initial={editing ?? null}
        orgId={scope.orgId}
        projects={scope.projects}
        onClose={() => setEditing(undefined)}
        onDone={() => { setEditing(undefined); void invalidate(); }}
      />
    </div>
  );
}

function PluginLoading() {
  return <div className="mx-auto grid w-full max-w-[1180px] gap-3 p-[22px] md:grid-cols-3">{[0, 1, 2].map((item) => <Skeleton key={item} width="100%" height={220} radius={10} />)}</div>;
}

function StageLane({ stage, plugins, projectNames, pending, onToggle, onEdit, onDelete }: {
  stage: (typeof STAGES)[number];
  plugins: PluginInstanceRow[];
  projectNames: Record<string, string>;
  pending: boolean;
  onToggle: (plugin: PluginInstanceRow) => void;
  onEdit: (plugin: PluginInstanceRow) => void;
  onDelete: (plugin: PluginInstanceRow) => void;
}) {
  return (
    <section className="grid gap-3 rounded-[12px] border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)]/30 p-4 lg:grid-cols-[210px_1fr]">
      <div>
        <div className="flex items-center gap-2"><span className="flex h-8 w-8 items-center justify-center rounded-lg border border-[color:var(--border-default)] bg-background"><Webhook className="h-4 w-4" aria-hidden /></span><h2 className="text-sm font-semibold">{stage.label}</h2></div>
        <p className="mt-2 text-xs leading-relaxed text-muted-foreground">{stage.description}</p>
      </div>
      {plugins.length === 0 ? <div className="flex min-h-[116px] items-center justify-center rounded-[10px] border border-dashed border-[color:var(--border-default)] text-xs text-[color:var(--text-subtle)]">No plugin configured at this stage</div> : (
        <div className="min-w-0 grid gap-3 md:grid-cols-2">
          {plugins.map((plugin) => (
            <article key={plugin.id} className="min-w-0 rounded-[10px] border border-[color:var(--border-default)] bg-background p-4">
              <div className="flex items-start gap-3"><div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-2"><h3 className="truncate text-sm font-semibold">{plugin.position.toString().padStart(2, "0")} · {plugin.name}</h3><Badge tone={plugin.enabled ? "accent" : "neutral"} dot>{plugin.enabled ? "desired on" : "paused"}</Badge></div><p className="mt-1 truncate font-mono text-xs text-muted-foreground">{plugin.endpoint}</p></div><Switch checked={plugin.enabled} disabled={pending} aria-label={`Enable ${plugin.name}`} onCheckedChange={() => onToggle(plugin)} /></div>
              <p className="mt-3 line-clamp-2 text-xs text-muted-foreground">{plugin.description || "No description"}</p>
              <div className="mt-3 flex flex-wrap gap-1.5"><Badge tone="outline">{plugin.project_id ? projectNames[plugin.project_id] ?? "project" : "organization"}</Badge><Badge tone={plugin.failure_mode === "fail_closed" ? "danger" : "warning"}>{plugin.failure_mode.replace("_", "-")}</Badge>{plugin.secret_env && <Badge tone="info">secret ref</Badge>}</div>
              <div className="mt-4 flex justify-end gap-2 border-t border-[color:var(--border-subtle)] pt-3"><Button variant="ghost" onClick={() => onDelete(plugin)}>Delete</Button><Button variant="outline" onClick={() => onEdit(plugin)}>Configure</Button></div>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function PluginDialog({ open, initial, orgId, projects, onClose, onDone }: {
  open: boolean;
  initial: PluginInstanceRow | null;
  orgId: string;
  projects: Array<{ id: string; name: string }>;
  onClose: () => void;
  onDone: () => void;
}) {
  const [form, setForm] = React.useState({
    project_id: initial?.project_id ?? "",
    name: initial?.name ?? "",
    slug: initial?.slug ?? "",
    description: initial?.description ?? "",
    stage: initial?.stage ?? "pre_route" as Stage,
    enabled: initial?.enabled ?? false,
    position: String(initial?.position ?? 0),
    failure_mode: initial?.failure_mode ?? "fail_open" as PluginInstanceRow["failure_mode"],
    endpoint: initial?.endpoint ?? "https://plugins.internal/hook",
    secret_env: initial?.secret_env ?? "",
    config: JSON.stringify(initial?.config ?? {}, null, 2),
  });
  const [localError, setLocalError] = React.useState<string | null>(null);
  const mutation = useMutation({
    mutationFn: (body: PluginInstanceInput) => initial ? updatePlugin(initial.id, body) : createPlugin(orgId, body),
    onSuccess: onDone,
  });
  const set = (patch: Partial<typeof form>) => { setForm((value) => ({ ...value, ...patch })); setLocalError(null); };
  const save = () => {
    let config: unknown;
    try { config = JSON.parse(form.config); } catch { setLocalError("Configuration must be valid JSON."); return; }
    if (!config || Array.isArray(config) || typeof config !== "object") { setLocalError("Configuration must be a JSON object."); return; }
    if (!form.name.trim() || !/^https?:\/\//.test(form.endpoint)) { setLocalError("Name and an HTTP(S) endpoint are required."); return; }
    mutation.mutate({ project_id: form.project_id || null, name: form.name, ...(initial ? { slug: initial.slug } : form.slug ? { slug: form.slug } : {}), description: form.description, kind: "webhook", stage: form.stage, enabled: form.enabled, position: Number(form.position), failure_mode: form.failure_mode, endpoint: form.endpoint, secret_env: form.secret_env || null, config: config as Record<string, unknown> });
  };
  return (
    <Dialog open={open} onOpenChange={(value) => !value && onClose()}>
      <DialogHeader><DialogTitle>{initial ? "Configure plugin" : "Install webhook plugin"}</DialogTitle><DialogDescription>Save the desired pipeline position and behavior. Secret values remain in gateway environment variables.</DialogDescription></DialogHeader>
      <div className="max-h-[65vh] space-y-4 overflow-y-auto pr-1">
        <div className="grid gap-3 sm:grid-cols-2"><Field label="Name" htmlFor="plugin-name"><Input id="plugin-name" value={form.name} onChange={(event) => set({ name: event.target.value })} /></Field><Field label="Slug" htmlFor="plugin-slug" hint={initial ? "Immutable after install." : "Derived from name when blank."}><Input id="plugin-slug" disabled={Boolean(initial)} value={form.slug} onChange={(event) => set({ slug: event.target.value })} /></Field></div>
        <Field label="Description" htmlFor="plugin-description"><Input id="plugin-description" value={form.description} onChange={(event) => set({ description: event.target.value })} /></Field>
        <div className="grid gap-3 sm:grid-cols-2"><Field label="Scope" htmlFor="plugin-scope"><Select id="plugin-scope" value={form.project_id} onChange={(event) => set({ project_id: event.target.value })}><option value="">Organization-wide</option>{projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}</Select></Field><Field label="Pipeline stage" htmlFor="plugin-stage"><Select id="plugin-stage" value={form.stage} onChange={(event) => set({ stage: event.target.value as Stage })}>{STAGES.map((stage) => <option key={stage.key} value={stage.key}>{stage.label}</option>)}</Select></Field></div>
        <Field label="Webhook endpoint" htmlFor="plugin-endpoint"><Input id="plugin-endpoint" type="url" value={form.endpoint} onChange={(event) => set({ endpoint: event.target.value })} /></Field>
        <div className="grid gap-3 sm:grid-cols-2"><Field label="Position" htmlFor="plugin-position" hint="Lower values run first."><Input id="plugin-position" type="number" min={0} value={form.position} onChange={(event) => set({ position: event.target.value })} /></Field><Field label="Failure policy" htmlFor="plugin-failure"><Select id="plugin-failure" value={form.failure_mode} onChange={(event) => set({ failure_mode: event.target.value as PluginInstanceRow["failure_mode"] })}><option value="fail_open">Fail open</option><option value="fail_closed">Fail closed</option></Select></Field></div>
        <Field label="Secret environment variable" htmlFor="plugin-secret" hint="Store the variable name, never the secret value."><Input id="plugin-secret" className="font-mono" placeholder="ROLTER_PLUGIN_TOKEN" value={form.secret_env} onChange={(event) => set({ secret_env: event.target.value })} /></Field>
        <Field label="Plugin configuration" htmlFor="plugin-config" hint="A JSON object passed to the future dispatcher."><Textarea id="plugin-config" className="font-mono text-xs" rows={6} value={form.config} onChange={(event) => set({ config: event.target.value })} /></Field>
        <div className="flex items-start justify-between gap-4 rounded-lg border border-[color:var(--border-subtle)] p-3"><div><p className="text-sm font-medium">Desired enabled state</p><p className="mt-0.5 text-xs text-muted-foreground">Saved now; execution begins when the gateway plugin runtime ships.</p></div><Switch checked={form.enabled} aria-label="Desired enabled state" onCheckedChange={(enabled) => set({ enabled })} /></div>
        {(localError || mutation.isError) && <p role="alert" className="text-xs text-destructive">{localError ?? (mutation.error as Error).message}</p>}
      </div>
      <DialogFooter><Button variant="ghost" onClick={onClose}>Cancel</Button><Button disabled={mutation.isPending} onClick={save}>{mutation.isPending ? "Saving…" : initial ? "Save configuration" : "Install configuration"}</Button></DialogFooter>
    </Dialog>
  );
}
