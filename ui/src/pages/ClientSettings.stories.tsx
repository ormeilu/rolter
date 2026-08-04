import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import ClientSettings from "./ClientSettings";
import type { ClientSettingsDto } from "@/lib/api";

const RESERVED = ["authorization", "x-api-key", "host", "cookie"];

const BASE: ClientSettingsDto = {
  public_base_url: null,
  forwarded_headers: [],
  injected_headers: {},
  request_id_header: "x-request-id",
  updated_at: "2026-08-05T12:00:00Z",
  always_propagated: ["traceparent", "tracestate", "b3"],
  reserved: RESERVED,
};

const CONFIGURED: ClientSettingsDto = {
  ...BASE,
  public_base_url: "https://gateway.example.com",
  forwarded_headers: ["x-tenant-id"],
  injected_headers: { "x-partner-id": "acme" },
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
      <ClientSettings />
    </QueryClientProvider>
  );
}

const meta = {
  title: "Screens/ClientSettings",
  component: ClientSettings,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ClientSettings>;

export default meta;
type Story = StoryObj<typeof meta>;

// the shipped default: no base URL override, no header policy
export const Empty: Story = {
  render: () => <Harness fetchStub={async () => json(BASE)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText(/No headers injected/)).toBeVisible());
  },
};

export const Configured: Story = {
  render: () => <Harness fetchStub={async () => json(CONFIGURED)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByLabelText("Public base URL")).toHaveValue(
      "https://gateway.example.com",
    );
    await expect(canvas.getByLabelText("Injected header name 1")).toHaveValue(
      "x-partner-id",
    );
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

// forwarding the caller's Authorization header upstream would hand a provider
// credential to whoever asked, so the server rejects it and so does the screen
export const RejectsAReservedHeader: Story = {
  render: () => <Harness fetchStub={async () => json(BASE)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const forwarded = await canvas.findByLabelText("Forwarded request headers");
    await userEvent.type(forwarded, "authorization");
    await waitFor(() =>
      expect(canvas.getByText("'authorization' is managed by the gateway.")).toBeVisible(),
    );
    await expect(canvas.getByRole("button", { name: "Save Changes" })).toBeDisabled();
  },
};

export const RejectsANonHttpBaseUrl: Story = {
  render: () => <Harness fetchStub={async () => json(BASE)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const url = await canvas.findByLabelText("Public base URL");
    await userEvent.type(url, "gateway.example.com");
    await waitFor(() =>
      expect(
        canvas.getByText("Base URL must start with http:// or https://."),
      ).toBeVisible(),
    );
  },
};

// interaction: adding an injected header round-trips and confirms
export const SavesChanges: Story = {
  render: () => {
    const stub: FetchStub = async (_input, init) => {
      if (init?.method === "PUT") {
        return json({ ...BASE, ...JSON.parse(String(init.body)) });
      }
      return json(BASE);
    };
    return <Harness fetchStub={stub} />;
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "Add header" }));
    await userEvent.type(
      canvas.getByLabelText("Injected header name 1"),
      "x-partner-id",
    );
    await userEvent.type(canvas.getByLabelText("Injected header value 1"), "acme");
    await userEvent.click(canvas.getByRole("button", { name: "Save Changes" }));
    await waitFor(() =>
      expect(canvas.getByText("Client settings updated.")).toBeVisible(),
    );
    await expect(canvas.getByLabelText("Injected header value 1")).toHaveValue("acme");
  },
};
