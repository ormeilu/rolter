import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Boxes, Plus, Puzzle, Server, Settings2, ShieldCheck, Wrench } from "lucide-react";
import * as React from "react";

import { PageBody } from "@/components/screen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog as BaseDialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  createMcpServer,
  createMcpToolGroup,
  deleteMcpServer,
  deleteMcpToolGroup,
  fetchMcpLibrary,
  fetchMcpServers,
  fetchMcpSettings,
  fetchMcpToolGroups,
  updateMcpServer,
  updateMcpSettings,
  updateMcpToolGroup,
  type McpGatewaySettingsRow,
  type McpLibraryItem,
  type McpServerInput,
  type McpServerRow,
  type McpToolGroupRow,
  type McpToolRef,
} from "@/lib/api";
import { useScope } from "@/lib/scope";

const TRANSPORTS = ["streamable_http", "sse", "websocket"];
const slugify = (value: string) => value.toLowerCase().trim().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 63);
const lines = (value: string) => [...new Set(value.split(/[,\n]/).map((item) => item.trim()).filter(Boolean))];

function Dialog({ onClose, children }: { open?: boolean; onClose: () => void; children: React.ReactNode }) {
  return <BaseDialog open onOpenChange={(open) => !open && onClose()}>{children}</BaseDialog>;
}

function LoadingGrid({ count = 3 }: { count?: number }) {
  return <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">{Array.from({ length: count }, (_, index) => <Skeleton key={index} width="100%" height={190} radius={10} />)}</div>;
}

function QueryError({ error, retry }: { error: Error; retry: () => void }) {
  return <div role="alert" className="rounded-[10px] border border-[color:var(--danger-border)] bg-[color:var(--danger-surface)] p-5"><p className="font-medium text-[color:var(--danger-text)]">MCP configuration unavailable</p><p className="mt-1 text-sm text-muted-foreground">{error.message}</p><Button className="mt-4" variant="outline" onClick={retry}>Try again</Button></div>;
}

function PageLead({ eyebrow, children, action }: { eyebrow: string; children: React.ReactNode; action?: React.ReactNode }) {
  return <div className="flex flex-col gap-4 border-b border-[color:var(--border-subtle)] pb-5 sm:flex-row sm:items-end"><div className="min-w-0 flex-1"><p className="font-mono text-[10px] uppercase tracking-[0.24em] text-[color:var(--accent)]">{eyebrow}</p><div className="mt-2 text-sm leading-relaxed text-muted-foreground">{children}</div></div>{action}</div>;
}

function ToolBadges({ tools }: { tools: string[] }) {
  if (!tools.length) return <span className="text-xs text-[color:var(--text-subtle)]">No tools declared</span>;
  return <div className="flex flex-wrap gap-1.5">{tools.map((tool) => <Badge key={tool} tone="outline">{tool}</Badge>)}</div>;
}

function ConfirmDelete({ server, pending, error, onClose, onConfirm }: { server: McpServerRow; pending: boolean; error: Error | null; onClose: () => void; onConfirm: () => void }) {
  return <Dialog open onClose={onClose}><DialogHeader><DialogTitle>Delete {server.name}?</DialogTitle><DialogDescription>Deleting this server also removes every OAuth grant and token session attached to it. This cannot be undone.</DialogDescription></DialogHeader>{error && <p role="alert" className="text-sm text-[color:var(--danger-text)]">{error.message}</p>}<DialogFooter><Button variant="ghost" onClick={onClose}>Cancel</Button><Button variant="destructive" disabled={pending} onClick={onConfirm}>{pending ? "Deleting…" : "Delete server"}</Button></DialogFooter></Dialog>;
}

export function McpCatalog() {
  const { orgId } = useScope();
  const client = useQueryClient();
  const query = useQuery({ queryKey: ["mcp-servers", orgId], queryFn: () => fetchMcpServers(orgId as string), enabled: !!orgId, retry: false });
  const [editing, setEditing] = React.useState<McpServerRow | null | undefined>(undefined);
  const [deleting, setDeleting] = React.useState<McpServerRow | null>(null);
  const save = useMutation({
    mutationFn: async ({ initial, input }: { initial: McpServerRow | null; input: McpServerInput }) => initial ? updateMcpServer(initial.id, input) : createMcpServer(orgId as string, input),
    onSuccess: () => { void client.invalidateQueries({ queryKey: ["mcp-servers", orgId] }); setEditing(undefined); },
  });
  const remove = useMutation({ mutationFn: deleteMcpServer, onSuccess: () => { void client.invalidateQueries({ queryKey: ["mcp-servers", orgId] }); setDeleting(null); } });
  const toggle = useMutation({ mutationFn: (server: McpServerRow) => updateMcpServer(server.id, { ...server, enabled: !server.enabled }), onSuccess: () => void client.invalidateQueries({ queryKey: ["mcp-servers", orgId] }) });
  if (!orgId) return <PageBody><EmptyState icon={<Server />} title="Choose an organization" description="MCP servers are isolated by organization." /></PageBody>;
  return <PageBody>
    <PageLead eyebrow="live registry" action={<Button onClick={() => setEditing(null)}><Plus className="h-4 w-4" aria-hidden />Register server</Button>}>
      {query.data ? `${query.data.filter((server) => server.enabled).length} enabled servers · ${query.data.reduce((sum, server) => sum + server.tools.length, 0)} declared tools` : "Registered servers are projected into the gateway snapshot when enabled."}
    </PageLead>
    {query.isLoading ? <LoadingGrid /> : query.error ? <QueryError error={query.error} retry={() => void query.refetch()} /> : !query.data?.length ? <EmptyState icon={<Server />} title="No MCP servers registered" description="Register a custom endpoint or install a curated server from MCP Library." actions={<Button onClick={() => setEditing(null)}>Register server</Button>} /> : <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">{query.data.map((server) => <article key={server.id} className={`min-w-0 rounded-[10px] border border-[color:var(--border-default)] bg-card p-4 ${server.enabled ? "" : "opacity-60"}`}>
      <div className="flex items-start gap-3"><span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-[color:var(--border-default)] bg-[color:var(--surface-subtle)]"><Server className="h-4 w-4" aria-hidden /></span><div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-2"><h2 className="truncate font-mono text-sm font-semibold">{server.name}</h2><Badge tone={server.source === "library" ? "accent" : "neutral"}>{server.source}</Badge></div><p className="mt-1 truncate font-mono text-xs text-muted-foreground">{server.url}</p></div><Switch checked={server.enabled} aria-label={`Enable ${server.name}`} onCheckedChange={() => toggle.mutate(server)} /></div>
      <p className="mt-3 min-h-10 text-xs leading-relaxed text-muted-foreground">{server.description || "No description provided."}</p><div className="mt-3"><ToolBadges tools={server.tools} /></div>
      <div className="mt-4 flex items-center gap-2 border-t border-[color:var(--border-subtle)] pt-3"><Badge tone="info">{server.transport.replace("_", " ")}</Badge><span className="ml-auto flex gap-1"><Button variant="ghost" aria-label={`Delete ${server.name}`} onClick={() => setDeleting(server)}>Delete</Button><Button variant="outline" aria-label={`Configure ${server.name}`} onClick={() => setEditing(server)}>Configure</Button></span></div>
    </article>)}</div>}
    {editing !== undefined && <ServerDialog initial={editing} pending={save.isPending} error={save.error} onClose={() => setEditing(undefined)} onSave={(input) => save.mutate({ initial: editing, input })} />}
    {deleting && <ConfirmDelete server={deleting} pending={remove.isPending} error={remove.error} onClose={() => setDeleting(null)} onConfirm={() => remove.mutate(deleting.id)} />}
  </PageBody>;
}

function ServerDialog({ initial, pending, error, onClose, onSave }: { initial: McpServerRow | null; pending: boolean; error: Error | null; onClose: () => void; onSave: (input: McpServerInput) => void }) {
  const [form, setForm] = React.useState<McpServerInput>(initial ?? { name: "", slug: "", url: "", transport: "streamable_http", description: "", enabled: true, tools: [], source: "custom", required_scopes: [] });
  const [tools, setTools] = React.useState(form.tools.join("\n"));
  const [scopes, setScopes] = React.useState(form.required_scopes.join("\n"));
  const valid = form.name.trim() && (initial || slugify(form.slug || form.name)) && /^https?:\/\//.test(form.url);
  return <Dialog open onClose={onClose}><DialogHeader><DialogTitle>{initial ? "Configure MCP server" : "Register MCP server"}</DialogTitle><DialogDescription>Declare the endpoint and exact tool names exposed through the organization registry.</DialogDescription></DialogHeader><div className="grid gap-4 py-4">
    <div className="grid gap-3 sm:grid-cols-2"><Field label="Name" htmlFor="mcp-name"><Input id="mcp-name" value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} /></Field><Field label="Slug" htmlFor="mcp-slug" hint="Used in /mcp/{server}; immutable after registration."><Input id="mcp-slug" disabled={!!initial} value={initial?.slug ?? form.slug} onChange={(event) => setForm({ ...form, slug: event.target.value })} /></Field></div>
    <Field label="Endpoint URL" htmlFor="mcp-url"><Input id="mcp-url" value={form.url} onChange={(event) => setForm({ ...form, url: event.target.value })} /></Field>
    <div className="grid gap-3 sm:grid-cols-2"><Field label="Transport" htmlFor="mcp-transport"><Select id="mcp-transport" value={form.transport} onChange={(event) => setForm({ ...form, transport: event.target.value })}>{TRANSPORTS.map((transport) => <option key={transport}>{transport}</option>)}</Select></Field><div className="flex items-end"><label className="flex min-h-9 w-full items-center justify-between rounded-md border border-[color:var(--border-default)] px-3 text-sm"><span>Enabled in gateway</span><Switch checked={form.enabled} onCheckedChange={(enabled) => setForm({ ...form, enabled })} /></label></div></div>
    <Field label="Description" htmlFor="mcp-description"><Textarea id="mcp-description" rows={2} value={form.description} onChange={(event) => setForm({ ...form, description: event.target.value })} /></Field>
    <div className="grid gap-3 sm:grid-cols-2"><Field label="Tools" htmlFor="mcp-tools" hint="One tool name per line."><Textarea id="mcp-tools" rows={5} value={tools} onChange={(event) => setTools(event.target.value)} /></Field><Field label="Required OAuth scopes" htmlFor="mcp-scopes" hint="One scope per line."><Textarea id="mcp-scopes" rows={5} value={scopes} onChange={(event) => setScopes(event.target.value)} /></Field></div>
    {error && <p role="alert" className="text-sm text-[color:var(--danger-text)]">{error.message}</p>}
  </div><DialogFooter><Button variant="ghost" onClick={onClose}>Cancel</Button><Button disabled={!valid || pending} onClick={() => onSave({ ...form, slug: initial?.slug ?? slugify(form.slug || form.name), tools: lines(tools), required_scopes: lines(scopes) })}>{pending ? "Saving…" : initial ? "Save server" : "Register server"}</Button></DialogFooter></Dialog>;
}

export function McpLibrary() {
  const { orgId } = useScope();
  const client = useQueryClient();
  const query = useQuery({ queryKey: ["mcp-library", orgId], queryFn: () => fetchMcpLibrary(orgId as string), enabled: !!orgId, retry: false });
  const install = useMutation({ mutationFn: (item: McpLibraryItem) => createMcpServer(orgId as string, { ...item, enabled: true, source: "library" }), onSuccess: () => { void client.invalidateQueries({ queryKey: ["mcp-library", orgId] }); void client.invalidateQueries({ queryKey: ["mcp-servers", orgId] }); } });
  return <PageBody><PageLead eyebrow="curated endpoints">Install a reviewed server definition into this organization. OAuth consent remains per user; installation never stores a token.</PageLead>
    {query.isLoading ? <LoadingGrid count={4} /> : query.error ? <QueryError error={query.error} retry={() => void query.refetch()} /> : <div className="grid gap-3 md:grid-cols-2">{query.data?.map((item) => <article key={item.slug} className="rounded-[10px] border border-[color:var(--border-default)] bg-card p-5"><div className="flex items-start gap-3"><span className="flex h-10 w-10 items-center justify-center rounded-lg border border-[color:var(--border-default)] bg-[color:var(--surface-subtle)]"><Boxes className="h-5 w-5" aria-hidden /></span><div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-2"><h2 className="font-semibold">{item.name}</h2><Badge tone="info">{item.transport.replace("_", " ")}</Badge></div><p className="mt-1 text-sm text-muted-foreground">{item.description}</p></div></div><div className="mt-4"><ToolBadges tools={item.tools} /></div><div className="mt-5 flex items-center border-t border-[color:var(--border-subtle)] pt-4"><span className="text-xs text-muted-foreground">{item.required_scopes.length ? `${item.required_scopes.length} OAuth scopes` : "No OAuth scopes"}</span><Button className="ml-auto" variant={item.installed ? "outline" : "default"} disabled={item.installed || install.isPending} onClick={() => install.mutate(item)}>{item.installed ? "Installed" : "Install"}</Button></div></article>)}</div>}
  </PageBody>;
}

export function ToolGroups() {
  const { orgId } = useScope();
  const client = useQueryClient();
  const groups = useQuery({ queryKey: ["mcp-tool-groups", orgId], queryFn: () => fetchMcpToolGroups(orgId as string), enabled: !!orgId, retry: false });
  const servers = useQuery({ queryKey: ["mcp-servers", orgId], queryFn: () => fetchMcpServers(orgId as string), enabled: !!orgId, retry: false });
  const [editing, setEditing] = React.useState<McpToolGroupRow | null | undefined>(undefined);
  const save = useMutation({ mutationFn: ({ initial, input }: { initial: McpToolGroupRow | null; input: Omit<McpToolGroupRow, "id" | "org_id" | "created_at" | "updated_at"> }) => initial ? updateMcpToolGroup(initial.id, input) : createMcpToolGroup(orgId as string, input), onSuccess: () => { void client.invalidateQueries({ queryKey: ["mcp-tool-groups", orgId] }); setEditing(undefined); } });
  const remove = useMutation({ mutationFn: deleteMcpToolGroup, onSuccess: () => void client.invalidateQueries({ queryKey: ["mcp-tool-groups", orgId] }) });
  const serverName = (id: string) => servers.data?.find((server) => server.id === id)?.name ?? id.slice(0, 8);
  return <PageBody><PageLead eyebrow="governed manifests" action={<Button disabled={!servers.data?.length} onClick={() => setEditing(null)}><Plus className="h-4 w-4" aria-hidden />Create group</Button>}>Bundle exact server/tool pairs into reusable policy manifests. Groups define intended access; project assignment and gateway enforcement remain separate policy work.</PageLead>
    {groups.isLoading || servers.isLoading ? <LoadingGrid /> : groups.error ? <QueryError error={groups.error} retry={() => void groups.refetch()} /> : !groups.data?.length ? <EmptyState icon={<Puzzle />} title="No tool groups" description={servers.data?.length ? "Create a governed bundle from registered server tools." : "Register MCP servers and declare their tools before creating a group."} actions={servers.data?.length ? <Button onClick={() => setEditing(null)}>Create group</Button> : undefined} /> : <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">{groups.data.map((group) => <article key={group.id} className="rounded-[10px] border border-[color:var(--border-default)] bg-card p-4"><div className="flex items-center gap-2"><Puzzle className="h-4 w-4 text-[color:var(--accent)]" aria-hidden /><h2 className="font-semibold">{group.name}</h2><Badge className="ml-auto" tone={group.enabled ? "success" : "neutral"} dot>{group.enabled ? "enabled" : "paused"}</Badge></div><p className="mt-2 min-h-10 text-xs leading-relaxed text-muted-foreground">{group.description || "No description provided."}</p><div className="mt-3 flex flex-wrap gap-1.5">{group.tools.map((item) => <Badge key={`${item.server_id}/${item.tool}`} tone="outline">{serverName(item.server_id)}/{item.tool}</Badge>)}</div><div className="mt-4 flex justify-end gap-1 border-t border-[color:var(--border-subtle)] pt-3"><Button variant="ghost" onClick={() => remove.mutate(group.id)}>Delete</Button><Button variant="outline" onClick={() => setEditing(group)}>Configure</Button></div></article>)}</div>}
    {editing !== undefined && <ToolGroupDialog initial={editing} servers={servers.data ?? []} pending={save.isPending} error={save.error} onClose={() => setEditing(undefined)} onSave={(input) => save.mutate({ initial: editing, input })} />}
  </PageBody>;
}

function ToolGroupDialog({ initial, servers, pending, error, onClose, onSave }: { initial: McpToolGroupRow | null; servers: McpServerRow[]; pending: boolean; error: Error | null; onClose: () => void; onSave: (input: Omit<McpToolGroupRow, "id" | "org_id" | "created_at" | "updated_at">) => void }) {
  const [name, setName] = React.useState(initial?.name ?? ""); const [description, setDescription] = React.useState(initial?.description ?? ""); const [enabled, setEnabled] = React.useState(initial?.enabled ?? true); const [selected, setSelected] = React.useState<McpToolRef[]>(initial?.tools ?? []);
  const toggle = (item: McpToolRef) => setSelected((current) => current.some((tool) => tool.server_id === item.server_id && tool.tool === item.tool) ? current.filter((tool) => tool.server_id !== item.server_id || tool.tool !== item.tool) : [...current, item]);
  return <Dialog open onClose={onClose}><DialogHeader><DialogTitle>{initial ? "Configure tool group" : "Create tool group"}</DialogTitle><DialogDescription>Select exact tools. A group is a policy manifest, not a credential.</DialogDescription></DialogHeader><div className="grid gap-4 py-4"><div className="grid gap-3 sm:grid-cols-2"><Field label="Name" htmlFor="group-name"><Input id="group-name" value={name} onChange={(event) => setName(event.target.value)} /></Field><label className="mt-auto flex min-h-9 items-center justify-between rounded-md border border-[color:var(--border-default)] px-3 text-sm"><span>Enabled</span><Switch checked={enabled} onCheckedChange={setEnabled} /></label></div><Field label="Description" htmlFor="group-description"><Textarea id="group-description" rows={2} value={description} onChange={(event) => setDescription(event.target.value)} /></Field><fieldset><legend className="text-sm font-medium">Tools</legend><div className="mt-2 max-h-64 space-y-3 overflow-y-auto rounded-[10px] border border-[color:var(--border-default)] p-3">{servers.map((server) => <div key={server.id}><p className="mb-2 font-mono text-xs text-muted-foreground">{server.name}</p><div className="flex flex-wrap gap-2">{server.tools.map((tool) => { const active = selected.some((item) => item.server_id === server.id && item.tool === tool); return <Button key={tool} type="button" variant={active ? "default" : "outline"} aria-pressed={active} onClick={() => toggle({ server_id: server.id, tool })}>{tool}</Button>; })}</div></div>)}</div></fieldset>{error && <p role="alert" className="text-sm text-[color:var(--danger-text)]">{error.message}</p>}</div><DialogFooter><Button variant="ghost" onClick={onClose}>Cancel</Button><Button disabled={!name.trim() || !selected.length || pending} onClick={() => onSave({ name, slug: initial?.slug ?? slugify(name), description, enabled, tools: selected })}>{pending ? "Saving…" : "Save group"}</Button></DialogFooter></Dialog>;
}

export function McpSettings() {
  const { orgId } = useScope(); const client = useQueryClient();
  const query = useQuery({ queryKey: ["mcp-settings", orgId], queryFn: () => fetchMcpSettings(orgId as string), enabled: !!orgId, retry: false });
  if (!orgId) return <PageBody><EmptyState icon={<Settings2 />} title="Choose an organization" description="MCP defaults are organization scoped." /></PageBody>;
  if (query.isLoading) return <PageBody><LoadingGrid count={2} /></PageBody>;
  if (query.error) return <PageBody><QueryError error={query.error} retry={() => void query.refetch()} /></PageBody>;
  return <McpSettingsForm initial={query.data as McpGatewaySettingsRow} onSaved={() => void client.invalidateQueries({ queryKey: ["mcp-settings", orgId] })} />;
}

function McpSettingsForm({ initial, onSaved }: { initial: McpGatewaySettingsRow; onSaved: () => void }) {
  const { orgId } = useScope(); const [form, setForm] = React.useState(initial); const set = (patch: Partial<McpGatewaySettingsRow>) => setForm((current) => ({ ...current, ...patch }));
  const save = useMutation({ mutationFn: () => updateMcpSettings(orgId as string, form), onSuccess: (next) => { setForm(next); onSaved(); } });
  return <PageBody><PageLead eyebrow="organization defaults">Defaults apply when registering new MCP servers. Runtime authorization still comes from server scopes and each user&apos;s OAuth session.</PageLead><div className="grid gap-4 lg:grid-cols-2"><section className="rounded-[10px] border border-[color:var(--border-default)] bg-card p-5"><div className="flex items-center gap-2"><Settings2 className="h-4 w-4 text-[color:var(--accent)]" aria-hidden /><h2 className="font-semibold">Transport defaults</h2></div><div className="mt-5 grid gap-4"><Field label="Default transport" htmlFor="default-transport"><Select id="default-transport" value={form.default_transport} onChange={(event) => set({ default_transport: event.target.value })}>{TRANSPORTS.map((transport) => <option key={transport}>{transport}</option>)}</Select></Field><Field label="Default failure policy" htmlFor="failure-mode" hint="Stored as the intended policy for clients that consume MCP settings."><Select id="failure-mode" value={form.default_failure_mode} onChange={(event) => set({ default_failure_mode: event.target.value as McpGatewaySettingsRow["default_failure_mode"] })}><option value="fail_closed">Fail closed</option><option value="fail_open">Fail open</option></Select></Field><label className="flex items-start justify-between gap-4 rounded-[10px] border border-[color:var(--border-subtle)] p-3"><span><span className="block text-sm font-medium">Allow unlisted tools</span><span className="mt-1 block text-xs leading-relaxed text-muted-foreground">Permit clients to request tools not declared in the registry manifest.</span></span><Switch checked={form.allow_unlisted_tools} onCheckedChange={(value) => set({ allow_unlisted_tools: value })} /></label></div></section><section className="rounded-[10px] border border-[color:var(--border-default)] bg-card p-5"><div className="flex items-center gap-2"><ShieldCheck className="h-4 w-4 text-[color:var(--accent)]" aria-hidden /><h2 className="font-semibold">Request controls</h2></div><div className="mt-5 grid gap-4 sm:grid-cols-2"><Field label="Connect timeout (ms)" htmlFor="connect-timeout"><Input id="connect-timeout" type="number" min={100} max={60000} value={form.connect_timeout_ms} onChange={(event) => set({ connect_timeout_ms: Number(event.target.value) })} /></Field><Field label="Request timeout (ms)" htmlFor="request-timeout"><Input id="request-timeout" type="number" min={1000} max={300000} value={form.request_timeout_ms} onChange={(event) => set({ request_timeout_ms: Number(event.target.value) })} /></Field><Field label="Maximum retries" htmlFor="max-retries"><Input id="max-retries" type="number" min={0} max={5} value={form.max_retries} onChange={(event) => set({ max_retries: Number(event.target.value) })} /></Field></div><p className="mt-5 flex items-start gap-2 rounded-[10px] border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] p-3 text-xs leading-relaxed text-muted-foreground"><Wrench className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />These values are persisted for MCP-aware clients. The current HTTP proxy continues to use deployment-level transport timeouts.</p></section></div>{save.error && <p role="alert" className="text-sm text-[color:var(--danger-text)]">{save.error.message}</p>}<div className="flex justify-end"><Button disabled={save.isPending} onClick={() => save.mutate()}>{save.isPending ? "Saving…" : "Save MCP settings"}</Button></div></PageBody>;
}
