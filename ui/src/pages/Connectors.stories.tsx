import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, waitFor, within } from "storybook/test";

import Connectors from "./Connectors";
import type { ConnectorRow } from "@/lib/api";

const connector = (over: Partial<ConnectorRow> = {}): ConnectorRow => ({
  id: "c-1",
  name: "signoz",
  kind: "otlp_http",
  endpoint: "https://collector.example.com/v1/logs",
  enabled: true,
  sampling_rate: 1,
  auth_secret_ref: null,
  auth_secret_configured: true,
  health_status: "healthy",
  // fixed rather than relative: nothing here should depend on wall-clock time
  health_checked_at: "2026-08-06T10:00:00Z",
  health_error: null,
  created_at: "2026-08-01T10:00:00Z",
  updated_at: "2026-08-06T10:00:00Z",
  ...over,
});

const CONNECTORS: ConnectorRow[] = [
  connector(),
  // configured but never turned on — the default state, since connectors are
  // strictly opt-in
  connector({
    id: "c-2",
    name: "honeycomb",
    enabled: false,
    sampling_rate: 0.1,
    health_status: "unknown",
    health_checked_at: null,
    auth_secret_configured: false,
  }),
  // an endpoint that answered, badly
  connector({
    id: "c-3",
    name: "datadog-staging",
    health_status: "unhealthy",
    health_error: "sink returned HTTP 401",
  }),
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
      <Connectors />
    </QueryClientProvider>
  );
}

const meta = {
  title: "Screens/Connectors",
  component: Connectors,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Connectors>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => <Harness fetchStub={async () => json(CONNECTORS)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("signoz")).toBeVisible());

    // health is its own axis, independent of enabled: a connector that has
    // never been tested reports `unknown` rather than claiming to be healthy
    await expect(canvas.getByText("healthy")).toBeVisible();
    await expect(canvas.getByText("unknown")).toBeVisible();
    await expect(canvas.getByText("unhealthy")).toBeVisible();

    // the sampling rate is shown as a percentage, so 0.1 must read as 10%
    await expect(canvas.getByText("10% sampled")).toBeVisible();

    // a failure names the status, never the sink's response body
    await expect(canvas.getByText(/HTTP 401/)).toBeVisible();
  },
};

export const Loading: Story = {
  render: () => <Harness fetchStub={() => new Promise<Response>(() => {})} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("Loading…")).toBeVisible();
  },
};

// the default for every deployment: connectors are opt-in, so an untouched
// install has none and no egress path at all
export const Empty: Story = {
  render: () => <Harness fetchStub={async () => json([])} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText(/No connectors yet/)).toBeVisible(),
    );
  },
};

// connectors are a deployment-wide egress decision, so a non-superadmin gets 403
export const Error_: Story = {
  name: "Error",
  render: () => (
    <Harness fetchStub={async () => json({ error: { message: "forbidden" } }, 403)} />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText(/need superadmin access/)).toBeVisible(),
    );
  },
};
