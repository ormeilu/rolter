import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import AccessProfiles from "./AccessProfiles";
import type { AccessProfileRow } from "@/lib/api";

const ORG = "11111111-1111-1111-1111-111111111111";

const profile = (over: Partial<AccessProfileRow> = {}): AccessProfileRow => ({
  id: "p-1",
  org_id: ORG,
  slug: "support-engineers",
  name: "Support engineers",
  description: "Read-only access to logs and analytics",
  created_at: "2026-08-01T10:00:00Z",
  updated_at: "2026-08-01T10:00:00Z",
  ...over,
});

const PROFILES: AccessProfileRow[] = [
  profile(),
  // a profile that reaches nobody grants nothing — the screen says so rather
  // than showing "0 users · 0 teams"
  profile({
    id: "p-2",
    slug: "oncall",
    name: "On-call",
    description: null,
  }),
];

const ASSIGNMENTS: Record<string, unknown[]> = {
  "p-1": [
    { id: "a-1", profile_id: "p-1", user_id: "u-1", team_id: null, created_at: "" },
    { id: "a-2", profile_id: "p-1", user_id: null, team_id: "t-1", created_at: "" },
  ],
  "p-2": [],
};

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

/// Route by URL: the screen resolves an org through `useScope`, which fetches
/// `/api/v1/orgs` first, so a stub that answers only the profile call would
/// leave the screen permanently disabled.
function stub(profiles: () => Promise<Response>): FetchStub {
  return async (input) => {
    const url = String(input);
    if (url.includes("/access-profiles/") && url.endsWith("/assignments")) {
      const id = url.split("/access-profiles/")[1].split("/")[0];
      return json(ASSIGNMENTS[id] ?? []);
    }
    if (url.includes("/access-profiles")) return profiles();
    if (url.includes("/teams") || url.includes("/projects")) return json([]);
    if (url.includes("/orgs")) return json([{ id: ORG, name: "Acme" }]);
    return json([]);
  };
}

// the stub is installed during render, not in an effect: child effects run
// before the parent's, so an effect would let the first real fetch through
function Harness({ fetchStub }: { fetchStub: FetchStub }) {
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
  return (
    <QueryClientProvider client={client}>
      <AccessProfiles />
    </QueryClientProvider>
  );
}

const meta = {
  title: "Screens/AccessProfiles",
  component: AccessProfiles,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof AccessProfiles>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => <Harness fetchStub={stub(async () => json(PROFILES))} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText("Support engineers")).toBeVisible(),
    );
    await expect(canvas.getByText("On-call")).toBeVisible();

    // a profile's reach is the thing that matters: one user plus one team
    await waitFor(() =>
      expect(canvas.getByText("1 user · 1 team")).toBeVisible(),
    );
    // and a profile assigned to nobody says so plainly
    await expect(canvas.getByText("Not assigned to anyone yet")).toBeVisible();
  },
};

export const Loading: Story = {
  render: () => (
    <Harness fetchStub={stub(() => new Promise<Response>(() => {}))} />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("Loading…")).toBeVisible());
  },
};

// the state every deployment starts in: the backend has shipped for a while,
// but nobody has created a profile yet
export const Empty: Story = {
  render: () => <Harness fetchStub={stub(async () => json([]))} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText("No access profiles yet")).toBeVisible(),
    );
  },
};

// the delete mutation is shared by every card, so `isPending` alone marks the
// whole grid busy. the pending row is the one the mutation was given, and this
// story is what stops that regression coming back: one row spins, the other
// stays clickable.
export const Deleting: Story = {
  render: () => (
    <Harness
      fetchStub={async (input, init) => {
        // hang the delete so the pending state stays on screen
        if (init?.method === "DELETE") return new Promise<Response>(() => {});
        return stub(async () => json(PROFILES))(input, init);
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText("Support engineers")).toBeVisible(),
    );

    const target = canvas.getByLabelText("Delete Support engineers");
    const other = canvas.getByLabelText("Delete On-call");
    await expect(target).toBeEnabled();
    await expect(other).toBeEnabled();

    await userEvent.click(target);

    // the clicked row goes busy...
    await waitFor(() => expect(target).toBeDisabled());
    // ...and the sibling row is untouched
    await expect(other).toBeEnabled();
  },
};

// profiles are org-scoped and admin-gated, so a viewer gets 403
export const Error_: Story = {
  name: "Error",
  render: () => (
    <Harness
      fetchStub={stub(async () => json({ error: { message: "forbidden" } }, 403))}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText(/need admin access/)).toBeVisible(),
    );
  },
};
