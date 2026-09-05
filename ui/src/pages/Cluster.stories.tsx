import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import Cluster from "./Cluster";
import {
  cancelConfirmation,
  confirmDestructive,
  expectSkeleton,
  recording,
} from "./story-harness";
import type { ClusterNodeRow } from "@/lib/api";

const node = (over: Partial<ClusterNodeRow> = {}): ClusterNodeRow => ({
  id: "gw-1",
  role: "gateway",
  build_version: "0.0.10",
  config_version: 7,
  desired_state: "active",
  state_changed_at: new Date().toISOString(),
  first_seen_at: new Date().toISOString(),
  last_seen_at: new Date().toISOString(),
  live: true,
  converged: true,
  ...over,
});

const FLEET: ClusterNodeRow[] = [
  node(),
  node({ id: "gw-2", config_version: 6, converged: false }),
  node({ id: "gw-3", desired_state: "draining" }),
  node({
    id: "gw-old",
    live: false,
    last_seen_at: new Date(Date.now() - 3_600_000).toISOString(),
  }),
  node({ id: "cp-1", role: "control" }),
];

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

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
      <Cluster />
    </QueryClientProvider>
  );
}

const meta = {
  title: "Screens/Cluster",
  component: Cluster,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Cluster>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => <Harness fetchStub={async () => json(FLEET)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // liveness and convergence are separate axes: gw-2 polls but lags
    await waitFor(() => expect(canvas.getByText("LAGGING")).toBeVisible());
    await expect(canvas.getByText("STALE")).toBeVisible();
    await expect(canvas.getByText("DRAINING")).toBeVisible();
  },
};

export const Loading: Story = {
  render: () => <Harness fetchStub={() => new Promise<Response>(() => {})} />,
  play: async ({ canvasElement }) => {
    await expectSkeleton(canvasElement);
  },
};

// a single-node deployment that sends no identity headers reports nothing
export const Empty: Story = {
  render: () => <Harness fetchStub={async () => json([])} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText("No nodes have reported in")).toBeVisible(),
    );
  },
};

// a non-superadmin principal gets 403
export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={async () => json({ error: { message: "forbidden" } }, 403)} />
  ),
};

// draining the only live gateway would take the data plane offline; the server
// refuses it and the screen surfaces the refusal rather than swallowing it
export const RefusesDrainingTheLastGateway: Story = {
  render: () => {
    const stub: FetchStub = async (_input, init) => {
      if (init?.method === "PUT") {
        return json(
          {
            error: {
              message:
                "node gw-1 is the only live gateway still serving; draining it would take the data plane offline",
            },
          },
          400,
        );
      }
      return json([node()]);
    };
    return <Harness fetchStub={stub} />;
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "Drain" }));
    await waitFor(() =>
      expect(canvas.getByText(/only live gateway still serving/)).toBeVisible(),
    );
  },
};

// forgetting a node that is still polling is pointless — it reappears on its
// next snapshot poll — so the action is only offered once it has gone stale
export const ForgetOnlyOfferedForStaleNodes: Story = {
  render: () => (
    <Harness
      fetchStub={async () => json([node(), node({ id: "gw-old", live: false })])}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const buttons = await canvas.findAllByRole("button", { name: "Forget" });
    await expect(buttons[0]).toBeDisabled();
    await expect(buttons[1]).toBeEnabled();
  },
};

// forgetting drops a node's history from the inventory, so it is confirmed by
// id — and the dialog repeats the thing that surprises people, that a node
// still running comes straight back (#1179)
const forgets = recording(async (_input, init) => {
  if (init?.method === "DELETE") return json({}, 204);
  return json([node(), node({ id: "gw-old", live: false })]);
});

const NODE_PATH = "/cluster/nodes/gw-old";

export const ConfirmsBeforeForgettingANode: Story = {
  render: () => <Harness fetchStub={forgets.stub} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const stale = async () =>
      (await canvas.findAllByRole("button", { name: "Forget" }))[1];

    await userEvent.click(await stale());
    await cancelConfirmation();
    forgets.expectNotSent("DELETE", NODE_PATH);

    await userEvent.click(await stale());
    await confirmDestructive(/gw-old/, /forget node/i);
    await forgets.expectSent("DELETE", NODE_PATH);
  },
};
