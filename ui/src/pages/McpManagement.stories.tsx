import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react-vite";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { McpCatalog, McpLibrary, McpSettings, ToolGroups } from "./McpManagement";
import type { McpGatewaySettingsRow, McpLibraryItem, McpServerRow, McpToolGroupRow } from "@/lib/api";

const ORG = { id: "org-1", name: "Acme", slug: "acme", created_at: "2026-01-01T00:00:00Z" };
const TEAM = { id: "team-1", org_id: ORG.id, name: "Platform", created_at: ORG.created_at };
const PROJECT = { id: "project-1", team_id: TEAM.id, name: "Gateway", created_at: ORG.created_at };
const SERVERS: McpServerRow[] = [
  { id: "server-github", org_id: ORG.id, name: "GitHub", slug: "github", url: "https://api.githubcopilot.com/mcp/", transport: "streamable_http", description: "Repository and pull request operations.", enabled: true, tools: ["search_code", "create_issue", "get_pull_request"], source: "library", required_scopes: ["repo"], created_at: ORG.created_at },
  { id: "server-sentry", org_id: ORG.id, name: "Sentry", slug: "sentry", url: "https://mcp.sentry.dev/mcp", transport: "streamable_http", description: "Production issue investigation.", enabled: false, tools: ["list_issues", "get_issue"], source: "custom", required_scopes: ["org:read"], created_at: ORG.created_at },
];
const LIBRARY: McpLibraryItem[] = [
  { slug: "github", name: "GitHub", description: "Repository, issue, pull request, and code search tools.", url: "https://api.githubcopilot.com/mcp/", transport: "streamable_http", tools: ["search_code", "create_issue"], required_scopes: ["repo"], installed: true },
  { slug: "linear", name: "Linear", description: "Browse and manage teams, issues, and projects.", url: "https://mcp.linear.app/mcp", transport: "streamable_http", tools: ["list_issues", "create_issue"], required_scopes: ["read", "write"], installed: false },
];
const GROUPS: McpToolGroupRow[] = [{ id: "group-1", org_id: ORG.id, name: "Triage", slug: "triage", description: "Read-only incident and code investigation tools.", enabled: true, tools: [{ server_id: "server-github", tool: "search_code" }, { server_id: "server-sentry", tool: "list_issues" }], created_at: ORG.created_at, updated_at: ORG.created_at }];
const SETTINGS: McpGatewaySettingsRow = { org_id: ORG.id, default_transport: "streamable_http", connect_timeout_ms: 5000, request_timeout_ms: 30000, max_retries: 1, default_failure_mode: "fail_closed", allow_unlisted_tools: false, updated_at: ORG.created_at };

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
const urlOf = (input: RequestInfo | URL) => typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;

function routed(over: Partial<Record<"servers" | "library" | "groups" | "settings", FetchStub>> = {}): FetchStub {
  return async (input, init) => {
    const url = urlOf(input);
    if (url.includes("/mcp/library")) return over.library?.(input, init) ?? json(LIBRARY);
    if (url.includes("/mcp/tool-groups")) return over.groups?.(input, init) ?? json(GROUPS);
    if (url.includes("/mcp/settings")) return over.settings?.(input, init) ?? json(SETTINGS);
    if (url.includes("/mcp-servers")) return over.servers?.(input, init) ?? json(SERVERS);
    if (url.includes("/projects")) return json([PROJECT]);
    if (url.includes("/teams")) return json([TEAM]);
    if (url.includes("/orgs")) return json([ORG]);
    return json([]);
  };
}

function Harness({ fetchStub, children }: { fetchStub: FetchStub; children: React.ReactNode }) {
  const original = React.useRef<typeof globalThis.fetch | null>(null);
  const client = React.useMemo(() => { original.current ??= globalThis.fetch; globalThis.fetch = fetchStub as typeof globalThis.fetch; return new QueryClient({ defaultOptions: { queries: { retry: false } } }); }, [fetchStub]);
  React.useEffect(() => () => { if (original.current) globalThis.fetch = original.current; }, []);
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

const meta = { title: "Screens/McpManagement", component: McpCatalog, parameters: { layout: "fullscreen" } } satisfies Meta<typeof McpCatalog>;
export default meta;
type Story = StoryObj<typeof meta>;

export const CatalogLoaded: Story = { render: () => <Harness fetchStub={routed()}><McpCatalog /></Harness>, play: async ({ canvasElement }) => { const canvas = within(canvasElement); await waitFor(() => expect(canvas.getByText("GitHub")).toBeVisible()); await expect(canvas.getByText("3 declared tools")).toBeVisible(); await expect(canvas.getByRole("switch", { name: "Enable Sentry" })).not.toBeChecked(); } };
export const CatalogLoading: Story = { render: () => <Harness fetchStub={routed({ servers: () => new Promise<Response>(() => {}) })}><McpCatalog /></Harness> };
export const CatalogEmpty: Story = { render: () => <Harness fetchStub={routed({ servers: async () => json([]) })}><McpCatalog /></Harness> };
export const CatalogForbidden: Story = { render: () => <Harness fetchStub={routed({ servers: async () => json({ error: { message: "forbidden" } }, 403) })}><McpCatalog /></Harness> };
export const CatalogValidatesEndpoint: Story = { render: () => <Harness fetchStub={routed()}><McpCatalog /></Harness>, play: async ({ canvasElement }) => { const canvas = within(canvasElement); await userEvent.click(await canvas.findByRole("button", { name: "Register server" })); const body = within(document.body); await userEvent.type(body.getByLabelText("Name"), "Local tools"); await userEvent.type(body.getByLabelText("Endpoint URL"), "ftp://invalid"); await expect(body.getByRole("button", { name: "Register server" })).toBeDisabled(); } };
export const CatalogExplainsDeleteCascade: Story = { render: () => <Harness fetchStub={routed()}><McpCatalog /></Harness>, play: async ({ canvasElement }) => { const canvas = within(canvasElement); await userEvent.click(await canvas.findByRole("button", { name: "Delete GitHub" })); const body = within(document.body); await expect(body.getByText(/removes every OAuth grant and token session/)).toBeVisible(); await expect(body.getByRole("button", { name: "Delete server" })).toBeEnabled(); } };

export const LibraryLoaded: Story = { render: () => <Harness fetchStub={routed()}><McpLibrary /></Harness> };
export const LibraryInstallsServer: Story = { render: () => { let installed = false; const stub = routed({ library: async () => json(LIBRARY.map((item) => item.slug === "linear" ? { ...item, installed } : item)), servers: async (_input, init) => { if (init?.method === "POST") { installed = true; return json({ ...SERVERS[0], id: "server-linear", name: "Linear", slug: "linear" }); } return json(SERVERS); } }); return <Harness fetchStub={stub}><McpLibrary /></Harness>; }, play: async ({ canvasElement }) => { const canvas = within(canvasElement); await userEvent.click(await canvas.findByRole("button", { name: "Install" })); await waitFor(() => expect(canvas.getAllByRole("button", { name: "Installed" })).toHaveLength(2)); } };

export const ToolGroupsLoaded: Story = { render: () => <Harness fetchStub={routed()}><ToolGroups /></Harness>, play: async ({ canvasElement }) => { const canvas = within(canvasElement); await waitFor(() => expect(canvas.getByText("Triage")).toBeVisible()); await expect(canvas.getByText("GitHub/search_code")).toBeVisible(); } };
export const ToolGroupsEmpty: Story = { render: () => <Harness fetchStub={routed({ groups: async () => json([]) })}><ToolGroups /></Harness> };
export const ToolGroupSelectsTools: Story = { render: () => <Harness fetchStub={routed({ groups: async () => json([]) })}><ToolGroups /></Harness>, play: async ({ canvasElement }) => { const canvas = within(canvasElement); await userEvent.click(await canvas.findByRole("button", { name: "Create group" })); const body = within(document.body); await userEvent.type(body.getByLabelText("Name"), "Builders"); await userEvent.click(body.getByRole("button", { name: "create_issue" })); await expect(body.getByRole("button", { name: "create_issue" })).toHaveAttribute("aria-pressed", "true"); await expect(body.getByRole("button", { name: "Save group" })).toBeEnabled(); } };

export const SettingsLoaded: Story = { render: () => <Harness fetchStub={routed()}><McpSettings /></Harness> };
export const SettingsSavesChanges: Story = { render: () => { let saved = SETTINGS; const stub = routed({ settings: async (_input, init) => { if (init?.method === "PUT") saved = { ...SETTINGS, ...JSON.parse(String(init.body)), updated_at: "2026-08-02T00:00:00Z" }; return json(saved); } }); return <Harness fetchStub={stub}><McpSettings /></Harness>; }, play: async ({ canvasElement }) => { const canvas = within(canvasElement); const retries = await canvas.findByLabelText("Maximum retries"); await userEvent.clear(retries); await userEvent.type(retries, "3"); await userEvent.click(canvas.getByRole("button", { name: "Save MCP settings" })); await waitFor(() => expect(retries).toHaveValue(3)); } };
