import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react-vite";
import * as React from "react";
import { expect, userEvent, waitFor } from "storybook/test";

import type { AdaptiveRoutingTelemetryDto } from "@/lib/api";
import AdaptiveDashboard from "./AdaptiveDashboard";

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

const TELEMETRY: AdaptiveRoutingTelemetryDto = {
  generated_at: "2026-08-02T01:15:00Z",
  fresh_window_secs: 60,
  routes: [
    {
      model: "gpt-4o",
      engaged: true,
      nodes: [
        {
          node_id: "gateway-eu-1",
          engaged: true,
          observed: 1_284,
          decisions: { blend: 1_056, exploration: 64, fallback: 164 },
          policy: {
            enabled: true,
            latency_weight: 0.55,
            cost_weight: 0.25,
            load_weight: 0.2,
            exploration_ratio: 0.05,
            min_samples: 100,
          },
          targets: [
            {
              target: 0,
              provider: "openai-primary",
              upstream_model: "gpt-4o-2024-11-20",
              score: 0.814,
              latency_score: 0.91,
              cost_score: 0.42,
              load_score: 0.72,
              latency_ms: 238.4,
              cost_per_mtok: 7.5,
              in_flight: 4,
              samples: 842,
              last_sample_age_ms: 1_200,
            },
            {
              target: 1,
              provider: "azure-west",
              upstream_model: "gpt-4o",
              score: 0.667,
              latency_score: 0.62,
              cost_score: 0.73,
              load_score: 0.7,
              latency_ms: 311.2,
              cost_per_mtok: 6.8,
              in_flight: 2,
              samples: 442,
              last_sample_age_ms: 850,
            },
          ],
          reported_at: "2026-08-02T01:14:52Z",
        },
        {
          node_id: "gateway-us-2",
          engaged: false,
          observed: 42,
          decisions: { blend: 0, exploration: 2, fallback: 40 },
          policy: {
            enabled: true,
            latency_weight: 0.55,
            cost_weight: 0.25,
            load_weight: 0.2,
            exploration_ratio: 0.05,
            min_samples: 100,
          },
          targets: [
            {
              target: 0,
              provider: "openai-us",
              upstream_model: "gpt-4o",
              score: 0.5,
              latency_ms: 0,
              cost_per_mtok: 7.5,
              in_flight: 0,
              samples: 42,
              last_sample_age_ms: 2_400,
            },
          ],
          reported_at: "2026-08-02T01:14:48Z",
        },
      ],
    },
    {
      model: "claude-sonnet",
      engaged: false,
      nodes: [
        {
          node_id: "gateway-eu-1",
          engaged: false,
          observed: 18,
          decisions: { blend: 0, exploration: 0, fallback: 18 },
          policy: {
            enabled: false,
            latency_weight: 1,
            cost_weight: 0,
            load_weight: 0,
            exploration_ratio: 0.02,
            min_samples: 50,
          },
          targets: [],
          reported_at: "2026-08-02T01:14:52Z",
        },
      ],
    },
  ],
};

// Install the deterministic fetch stub before child effects run, matching the
// other API-backed screen stories in this repository.
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
      <AdaptiveDashboard />
    </QueryClientProvider>
  );
}

const meta = {
  title: "Screens/AdaptiveDashboard",
  component: AdaptiveDashboard,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof AdaptiveDashboard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => <Harness fetchStub={async () => json(TELEMETRY)} />,
  play: async ({ canvas }) => {
    await waitFor(() => expect(canvas.getByText("BLEND ACTIVE")).toBeVisible());
    await expect(canvas.getAllByText("DISABLED")).toHaveLength(2);
    await expect(canvas.getByRole("cell", { name: "0.814" })).toBeVisible();

    // Additional gateways stay compact until the operator asks for their
    // node-specific signals. Native details/summary preserves keyboard access.
    await userEvent.click(canvas.getByText("gateway-us-2"));
    await expect(canvas.getByRole("rowheader", { name: "openai-us / gpt-4o" })).toBeVisible();
  },
};

export const Loading: Story = {
  render: () => <Harness fetchStub={() => new Promise<Response>(() => {})} />,
};

export const Empty: Story = {
  render: () => (
    <Harness
      fetchStub={async () =>
        json({ ...TELEMETRY, routes: [] } satisfies AdaptiveRoutingTelemetryDto)
      }
    />
  ),
  play: async ({ canvas }) => {
    await waitFor(() =>
      expect(canvas.getByText("No fresh adaptive routing telemetry")).toBeVisible(),
    );
    await expect(canvas.getByRole("link", { name: "Review routing rules" })).toHaveAttribute(
      "href",
      "/routing-rules",
    );
  },
};

export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={async () => json({ error: { message: "forbidden" } }, 403)} />
  ),
  play: async ({ canvas }) => {
    await waitFor(() =>
      expect(canvas.getByRole("alert")).toHaveTextContent(
        "Unable to load adaptive routing telemetry",
      ),
    );
  },
};

export const RetryAfterFailure: Story = {
  render: () => {
    let attempts = 0;
    return (
      <Harness
        fetchStub={async () => {
          attempts += 1;
          return attempts === 1
            ? json({ error: { message: "temporarily unavailable" } }, 503)
            : json(TELEMETRY);
        }}
      />
    );
  },
  play: async ({ canvas }) => {
    await userEvent.click(await canvas.findByRole("button", { name: "Try again" }));
    await waitFor(() => expect(canvas.getByText("BLEND ACTIVE")).toBeVisible());
  },
};
