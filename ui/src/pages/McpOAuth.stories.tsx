import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { AuthSessions, OAuthGrants } from "./McpOAuth";
import {
  cancelConfirmation,
  confirmDestructive,
  expectSkeleton,
  recording,
} from "./story-harness";
import type {
  McpOAuthGrantRow,
  McpOAuthSessionRow,
  McpServerRow,
  UserRow,
} from "@/lib/api";

const ORG = { id: "org-1", name: "acme", slug: "acme", created_at: "2026-01-01T00:00:00Z" };
const TEAM = { id: "team-1", org_id: ORG.id, name: "platform", created_at: "2026-01-01T00:00:00Z" };
const PROJECT = { id: "proj-1", team_id: TEAM.id, name: "gateway", created_at: "2026-01-01T00:00:00Z" };

const SERVERS: McpServerRow[] = [
  {
    id: "srv-github",
    org_id: ORG.id,
    name: "github",
    slug: "github",
    url: "https://mcp.example.com/github",
    transport: "streamable_http",
    description: "GitHub tools",
    enabled: true,
    tools: ["search_code"],
    source: "custom",
    required_scopes: ["repo"],
    created_at: "2026-02-01T09:00:00Z",
    authorize_url: "https://mcp.example.com/github/authorize",
    token_url: "https://mcp.example.com/github/token",
    client_id: "rolter-github",
    default_scopes: ["repo"],
    has_client_secret: true,
  },
  {
    id: "srv-jira",
    org_id: ORG.id,
    name: "jira",
    slug: "jira",
    url: "https://mcp.example.com/jira",
    transport: "sse",
    description: "Jira tools",
    enabled: true,
    tools: ["list_issues"],
    source: "custom",
    required_scopes: ["read"],
    created_at: "2026-02-03T09:00:00Z",
    authorize_url: "https://mcp.example.com/jira/authorize",
    token_url: "https://mcp.example.com/jira/token",
    client_id: "rolter-jira",
    default_scopes: [],
    has_client_secret: false,
  },
];

const USERS: UserRow[] = [
  { id: "user-ada", email: "ada@acme.dev", is_superadmin: false, created_at: "2026-01-02T00:00:00Z" },
  { id: "user-bo", email: "bo@acme.dev", is_superadmin: false, created_at: "2026-01-03T00:00:00Z" },
];

const grant = (over: Partial<McpOAuthGrantRow> = {}): McpOAuthGrantRow => ({
  id: "grant-ada-github",
  server_id: "srv-github",
  user_id: "user-ada",
  scopes: ["tools:read", "tools:call"],
  granted_at: "2026-06-01T10:15:00Z",
  revoked_at: null,
  revoked_by: null,
  active: true,
  ...over,
});

const session = (over: Partial<McpOAuthSessionRow> = {}): McpOAuthSessionRow => ({
  id: "sess-1",
  grant_id: "grant-ada-github",
  scopes: ["tools:read"],
  expires_at: new Date(Date.now() + 3_600_000).toISOString(),
  refresh_expires_at: new Date(Date.now() + 86_400_000).toISOString(),
  revoked_at: null,
  created_at: "2026-06-01T10:15:00Z",
  last_used_at: new Date(Date.now() - 300_000).toISOString(),
  has_refresh_token: true,
  ...over,
});

// two grants for ada on github and one for bo on jira: the github consent has
// two live sessions under it, which is what the revoke confirmation must name
const GRANTS: McpOAuthGrantRow[] = [
  grant(),
  grant({
    id: "grant-bo-jira",
    server_id: "srv-jira",
    user_id: "user-bo",
    scopes: ["tools:read"],
    granted_at: "2026-05-20T08:00:00Z",
    revoked_at: "2026-06-02T08:00:00Z",
    revoked_by: "user-bo",
    active: false,
  }),
];

const SESSIONS: McpOAuthSessionRow[] = [
  session(),
  session({ id: "sess-2", scopes: ["tools:call"] }),
  session({
    id: "sess-3",
    grant_id: "grant-bo-jira",
    expires_at: new Date(Date.now() - 600_000).toISOString(),
    refresh_expires_at: null,
    has_refresh_token: false,
    last_used_at: null,
  }),
];

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

// these screens read the active org through useScope, so a stub has to answer
// the scope queries as well as the two listings under test
function routed(
  over: {
    servers?: () => Response;
    grants?: () => Response;
    sessions?: () => Response;
    users?: () => Response;
  } = {},
): FetchStub {
  return async (input) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.toString()
          : input.url;
    if (url.includes("/mcp-servers")) return over.servers?.() ?? json(SERVERS);
    if (url.includes("/mcp/grants")) return over.grants?.() ?? json(GRANTS);
    if (url.includes("/mcp/sessions")) return over.sessions?.() ?? json(SESSIONS);
    if (url.includes("/users")) return over.users?.() ?? json(USERS);
    if (url.includes("/teams")) return json([TEAM]);
    if (url.includes("/projects")) return json([PROJECT]);
    if (url.includes("/orgs")) return json([ORG]);
    return json([]);
  };
}

// the stub is installed during render, not in an effect: child effects run
// before the parent's, so an effect would let the first real fetch through
function Harness({
  fetchStub,
  children,
}: {
  fetchStub: FetchStub;
  children: React.ReactNode;
}) {
  const original = React.useRef<typeof globalThis.fetch | null>(null);
  const client = React.useMemo(() => {
    original.current ??= globalThis.fetch;
    globalThis.fetch = fetchStub as typeof globalThis.fetch;
    return new QueryClient({ defaultOptions: { queries: { retry: false } } });
  }, [fetchStub]);
  React.useEffect(
    () => () => {
      if (original.current) globalThis.fetch = original.current;
    },
    [],
  );
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

const meta = {
  title: "Screens/McpOAuth",
  component: OAuthGrants,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof OAuthGrants>;

export default meta;
type Story = StoryObj<typeof meta>;

export const GrantsLoaded: Story = {
  render: () => (
    <Harness fetchStub={routed()}>
      <OAuthGrants />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("ada@acme.dev")).toBeVisible());
    // the revoked consent is kept so the audit trail survives
    await expect(canvas.getByText("REVOKED")).toBeVisible();
    await expect(canvas.getByText("2 live")).toBeVisible();
    // expired sessions are historical rows, not live revoke impact
    await expect(canvas.getByText("0 live")).toBeVisible();
  },
};

// the cascade is the whole point of a grant: the confirmation says how many
// sessions go with it before the click, not after
export const GrantRevokeNamesItsSessions: Story = {
  render: () => (
    <Harness fetchStub={routed()}>
      <OAuthGrants />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const revoke = await canvas.findByRole("button", {
      name: "Revoke consent for github",
    });
    await userEvent.click(revoke);
    // the dialog portals to document.body, not the canvas root
    const body = within(document.body);
    await waitFor(() =>
      expect(body.getByText(/also revokes 2 live sessions/)).toBeVisible(),
    );
    await expect(
      body.getByText(/in the same transaction/),
    ).toBeVisible();
  },
};

export const GrantsLoading: Story = {
  render: () => (
    <Harness fetchStub={() => new Promise<Response>(() => {})}>
      <OAuthGrants />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectSkeleton(canvasElement);
  },
};

export const GrantsEmpty: Story = {
  render: () => (
    <Harness fetchStub={routed({ grants: () => json([]), sessions: () => json([]) })}>
      <OAuthGrants />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText("No consent granted yet")).toBeVisible(),
    );
  },
};

// no membership at the org: the listing is refused rather than filtered
export const GrantsForbidden: Story = {
  render: () => (
    <Harness
      fetchStub={routed({
        grants: () => json({ error: { message: "forbidden" } }, 403),
      })}
    >
      <OAuthGrants />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText(/You do not have access to OAuth grants/)).toBeVisible(),
    );
  },
};

export const SessionsLoaded: Story = {
  render: () => (
    <Harness fetchStub={routed()}>
      <AuthSessions />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("EXPIRED")).toBeVisible());
    // renewability is a flag on the row; the refresh token is never in the payload
    await expect(canvas.getAllByText("RENEWABLE")).toHaveLength(2);
    await expect(canvas.getByText("never")).toBeVisible();
  },
};

// a member's listing only carries rows they own, and the users endpoint they
// are not allowed to read must not take the screen down with it
export const SessionsMemberScoped: Story = {
  render: () => (
    <Harness
      fetchStub={routed({
        grants: () => json([grant()]),
        sessions: () => json([session()]),
        users: () => json({ error: { message: "forbidden" } }, 403),
      })}
    >
      <AuthSessions />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("1 sessions · 1 live")).toBeVisible());
    // the owner falls back to a short id rather than an error
    await expect(canvas.getByText("user-ada")).toBeVisible();
    await expect(
      canvas.getByText(/members and viewers see — and may revoke — only their own/),
    ).toBeVisible();
  },
};

export const SessionsLoading: Story = {
  render: () => (
    <Harness fetchStub={() => new Promise<Response>(() => {})}>
      <AuthSessions />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectSkeleton(canvasElement);
  },
};

export const SessionsEmpty: Story = {
  render: () => (
    <Harness fetchStub={routed({ sessions: () => json([]) })}>
      <AuthSessions />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("No sessions yet")).toBeVisible());
  },
};

// grants on this screen have always confirmed before revoking; a session
// revoke went straight through on one click until #1179
const sessionRevokes = recording(async (input, init) =>
  init?.method === "DELETE" ? json(session({ revoked_at: new Date().toISOString() })) : routed()(input, init),
);

export const SessionRevokeConfirmsFirst: Story = {
  render: () => (
    <Harness fetchStub={sessionRevokes.stub}>
      <AuthSessions />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const revoke = async () =>
      (await canvas.findAllByRole("button", { name: "Revoke session on github" }))[0];

    await userEvent.click(await revoke());
    await cancelConfirmation();
    sessionRevokes.expectNotSent("DELETE", "/mcp/sessions/sess-1");

    await userEvent.click(await revoke());
    // the copy is the counterpart of the grant dialog: this one does *not*
    // cascade, and says so
    await confirmDestructive(/github/, /revoke session/i);
    await sessionRevokes.expectSent("DELETE", "/mcp/sessions/sess-1");
  },
};

export const SessionsForbidden: Story = {
  render: () => (
    <Harness
      fetchStub={routed({
        sessions: () => json({ error: { message: "forbidden" } }, 403),
      })}
    >
      <AuthSessions />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText(/You do not have access to auth sessions/)).toBeVisible(),
    );
  },
};
