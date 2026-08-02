import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react-vite";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import PromptRepository from "./PromptRepository";
import type {
  PromptTemplateRow,
  PromptTemplateScopeRow,
  PromptTemplateVersionRow,
} from "@/lib/api";

const ORG = "00000000-0000-4000-8000-000000000001";
const TEAM = "00000000-0000-4000-8000-000000000002";
const PROJECT = "00000000-0000-4000-8000-000000000003";
const TEMPLATE = "00000000-0000-4000-8000-000000000004";
const ROUTE = "00000000-0000-4000-8000-000000000005";
const KEY = "00000000-0000-4000-8000-000000000006";

const template: PromptTemplateRow = {
  id: TEMPLATE,
  org_id: ORG,
  name: "Support concierge",
  slug: "support-concierge",
  description: "Sets a consistent support voice and adds escalation context.",
  published_version: 2,
  created_at: "2026-07-28T09:00:00Z",
};

const versions: PromptTemplateVersionRow[] = [
  {
    template_id: TEMPLATE,
    version: 2,
    variables: [
      { name: "customer_name", required: true },
      { name: "tone", required: false, default: "calm and direct" },
    ],
    decorators: [
      {
        role: "system",
        position: "prepend",
        content: "You support {{ customer_name }}. Keep the tone {{ tone }}.",
      },
      {
        role: "assistant",
        position: "append",
        content: "If the issue remains unresolved, summarize the next action.",
      },
    ],
    created_at: "2026-07-30T12:15:00Z",
  },
  {
    template_id: TEMPLATE,
    version: 1,
    variables: [{ name: "customer_name", required: true }],
    decorators: [
      {
        role: "system",
        position: "prepend",
        content: "You are the support concierge for {{ customer_name }}.",
      },
    ],
    created_at: "2026-07-28T09:05:00Z",
  },
];

const scopes: PromptTemplateScopeRow[] = [
  {
    template_id: TEMPLATE,
    version: 2,
    scope_type: "project",
    scope_id: PROJECT,
    created_at: "2026-07-30T12:15:00Z",
  },
];

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function loadedStub(): FetchStub {
  let currentVersions = [...versions];
  return async (input, init) => {
    const url = String(input);
    if (url === "/api/v1/orgs") return json([{ id: ORG, name: "Northstar", slug: "northstar", created_at: "2026-01-01T00:00:00Z" }]);
    if (url.endsWith(`/orgs/${ORG}/teams`)) return json([{ id: TEAM, org_id: ORG, name: "Platform", created_at: "2026-01-01T00:00:00Z" }]);
    if (url.endsWith(`/teams/${TEAM}/projects`)) return json([{ id: PROJECT, team_id: TEAM, name: "Production", created_at: "2026-01-01T00:00:00Z" }]);
    if (url.endsWith(`/orgs/${ORG}/prompt-templates`)) return json([template]);
    if (url.endsWith(`/projects/${PROJECT}/routes`)) return json([{ id: ROUTE, project_id: PROJECT, model: "support", strategy: "round_robin", enabled: true, params: {}, param_policy: {}, created_at: "2026-01-01T00:00:00Z" }]);
    if (url.endsWith(`/projects/${PROJECT}/virtual-keys`)) return json([{ id: KEY, project_id: PROJECT, key_hash: "hash", key_prefix: "rlt_prod", name: "Support app", models: ["support"], disabled: false, created_at: "2026-01-01T00:00:00Z" }]);
    if (url.endsWith(`/prompt-templates/${TEMPLATE}/versions`) && init?.method === "POST") {
      const body = JSON.parse(String(init.body)) as Pick<PromptTemplateVersionRow, "variables" | "decorators">;
      const created = { template_id: TEMPLATE, version: 3, ...body, created_at: "2026-08-02T08:00:00Z" };
      currentVersions = [created, ...currentVersions];
      return json(created);
    }
    if (url.endsWith(`/prompt-templates/${TEMPLATE}/versions`)) return json(currentVersions);
    if (url.includes(`/prompt-templates/${TEMPLATE}/versions/`) && url.endsWith("/scopes")) {
      if (init?.method === "PUT") return json(JSON.parse(String(init.body)).scopes);
      return json(url.includes("/2/") ? scopes : []);
    }
    if (url.endsWith(`/prompt-templates/${TEMPLATE}/publish`)) return json({ ...template, published_version: JSON.parse(String(init?.body)).version });
    if (url.endsWith(`/prompt-templates/${TEMPLATE}/rollback`)) return json({ ...template, published_version: JSON.parse(String(init?.body)).version });
    return json({ error: { message: `unhandled story request: ${url}` } }, 500);
  };
}

function Harness({ fetchStub }: { fetchStub: FetchStub }) {
  const original = React.useRef<typeof globalThis.fetch | null>(null);
  const client = React.useMemo(() => {
    original.current ??= globalThis.fetch;
    globalThis.fetch = fetchStub as typeof globalThis.fetch;
    localStorage.removeItem("rolter.scope");
    return new QueryClient({ defaultOptions: { queries: { retry: false } } });
  }, [fetchStub]);
  React.useEffect(() => () => {
    if (original.current) globalThis.fetch = original.current;
  }, []);
  return <QueryClientProvider client={client}><div className="h-screen bg-[color:var(--surface-app)]"><PromptRepository /></div></QueryClientProvider>;
}

const meta = {
  title: "Screens/PromptRepository",
  component: PromptRepository,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof PromptRepository>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => <Harness fetchStub={loadedStub()} />,
};

export const Empty: Story = {
  render: () => {
    const stub = loadedStub();
    return <Harness fetchStub={async (input, init) => String(input).endsWith(`/orgs/${ORG}/prompt-templates`) ? json([]) : stub(input, init)} />;
  },
};

export const Loading: Story = {
  render: () => <Harness fetchStub={() => new Promise<Response>(() => {})} />,
};

export const Error: Story = {
  render: () => {
    const stub = loadedStub();
    return <Harness fetchStub={async (input, init) => String(input).endsWith(`/orgs/${ORG}/prompt-templates`) ? json({ error: { message: "database is unavailable" } }, 503) : stub(input, init)} />;
  },
};

export const RendersSamplesAndSavesDraft: Story = {
  render: () => <Harness fetchStub={loadedStub()} />,
  play: async ({ canvas }) => {
    const sample = await canvas.findByRole("textbox", { name: "Sample value for customer_name" });
    await userEvent.type(sample, "Aster Labs");
    await expect(canvas.getByText(/You support Aster Labs/)).toBeVisible();
    await userEvent.click(canvas.getByRole("button", { name: /Save as new draft/ }));
    await waitFor(() => expect(canvas.getByText(/Draft v3 saved/)).toBeVisible());
  },
};

export const ConfirmsRollback: Story = {
  render: () => <Harness fetchStub={loadedStub()} />,
  play: async ({ canvas, canvasElement }) => {
    await userEvent.click(await canvas.findByRole("button", { name: "Roll back to v1" }));
    const page = within(canvasElement.ownerDocument.body);
    await expect(page.getByRole("heading", { name: "Roll back to v1" })).toBeVisible();
    await expect(page.getByText(/changes the live version from v2/)).toBeVisible();
  },
};
