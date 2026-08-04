import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import ModelSettings from "./ModelSettings";
import type { ModelDefaultsDto } from "@/lib/api";

const BASE: ModelDefaultsDto = {
  enabled: false,
  default_model: null,
  default_temperature: null,
  default_top_p: null,
  default_max_tokens: null,
  updated_at: "2026-08-05T12:00:00Z",
};

const CONFIGURED: ModelDefaultsDto = {
  ...BASE,
  enabled: true,
  default_model: "gpt-4o-mini",
  default_temperature: 0.7,
  default_max_tokens: 2048,
};

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
      <ModelSettings />
    </QueryClientProvider>
  );
}

const meta = {
  title: "Screens/ModelSettings",
  component: ModelSettings,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ModelSettings>;

export default meta;
type Story = StoryObj<typeof meta>;

// the shipped default: nothing set, nothing applied
export const Empty: Story = {
  render: () => <Harness fetchStub={async () => json(BASE)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("inactive")).toBeVisible());
  },
};

export const Configured: Story = {
  render: () => <Harness fetchStub={async () => json(CONFIGURED)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("active")).toBeVisible());
    await expect(canvas.getByLabelText("Default model")).toHaveValue("gpt-4o-mini");
  },
};

export const Loading: Story = {
  render: () => <Harness fetchStub={() => new Promise<Response>(() => {})} />,
};

// a non-superadmin principal gets 403
export const Forbidden: Story = {
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

// enabled with nothing filled in is a no-op, and the screen says so rather
// than implying traffic is being changed
export const EnabledWithNoDefaults: Story = {
  render: () => <Harness fetchStub={async () => json({ ...BASE, enabled: true })} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("inactive")).toBeVisible());
    await expect(canvas.getByText(/No defaults are set yet/)).toBeVisible();
  },
};

// the value is forwarded to the provider, so an out-of-range one is blocked
// before it can fail every request
export const RejectsAnOutOfRangeTemperature: Story = {
  render: () => <Harness fetchStub={async () => json(CONFIGURED)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const temperature = await canvas.findByLabelText("Temperature");
    await userEvent.clear(temperature);
    await userEvent.type(temperature, "3");
    await waitFor(() =>
      expect(canvas.getByText("Temperature must be between 0 and 2.")).toBeVisible(),
    );
    await expect(canvas.getByRole("button", { name: "Save Changes" })).toBeDisabled();
  },
};

// interaction: a valid edit round-trips and confirms
export const SavesChanges: Story = {
  render: () => {
    const stub: FetchStub = async (_input, init) => {
      if (init?.method === "PUT") {
        return json({ ...CONFIGURED, ...JSON.parse(String(init.body)) });
      }
      return json(CONFIGURED);
    };
    return <Harness fetchStub={stub} />;
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const tokens = await canvas.findByLabelText("Max tokens");
    await userEvent.clear(tokens);
    await userEvent.type(tokens, "4096");
    await userEvent.click(canvas.getByRole("button", { name: "Save Changes" }));
    await waitFor(() =>
      expect(canvas.getByText("Model defaults updated.")).toBeVisible(),
    );
    await expect(canvas.getByLabelText("Max tokens")).toHaveValue("4096");
  },
};
