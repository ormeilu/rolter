import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { ProviderSheet } from "./ProviderSheet";
import type { ProviderRow, ProviderTestResult } from "@/lib/api";

const PROVIDER: ProviderRow = {
  id: "prov-1",
  org_id: "11111111-1111-1111-1111-111111111111",
  name: "openai-primary",
  slug: "openai-primary",
  kind: "openai",
  api_base: "https://api.openai.com",
  api_key_env: null,
  egress_proxy: null,
  created_at: "2026-08-01T10:00:00Z",
};

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

/// Answer the probe with a fixed outcome; everything else is inert. The sheet
/// only calls the network when the operator presses the button.
function stub(test: () => Promise<Response>): FetchStub {
  return async (input) => {
    if (String(input).endsWith("/test")) return test();
    return json({});
  };
}

// installed during render, not in an effect: child effects run before the
// parent's, so an effect would let the first real fetch through
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
      <ProviderSheet
        open
        mode="edit"
        onOpenChange={() => {}}
        orgId={PROVIDER.org_id}
        provider={PROVIDER}
        onDone={() => {}}
      />
    </QueryClientProvider>
  );
}

const result = (over: Partial<ProviderTestResult> = {}): ProviderTestResult => ({
  reachable: true,
  probed_url: "https://api.openai.com/v1/models",
  status: 200,
  latency_ms: 142,
  credential: "stored",
  models_found: 38,
  error: null,
  ...over,
});

// every story renders through `Harness`, which owns the props; these satisfy the
// component's required-prop contract for the docs page and are not otherwise read
const meta = {
  title: "Overlays/ProviderSheet",
  component: ProviderSheet,
  parameters: { layout: "fullscreen" },
  args: {
    open: true,
    mode: "edit" as const,
    onOpenChange: () => {},
    orgId: PROVIDER.org_id,
    provider: PROVIDER,
    onDone: () => {},
  },
} satisfies Meta<typeof ProviderSheet>;

export default meta;
type Story = StoryObj<typeof meta>;

// the Sheet renders through a portal onto document.body, so the story's
// canvasElement is empty — query the whole document instead
const screen = () => within(document.body);

const press = async () =>
  userEvent.click(await screen().findByRole("button", { name: /test connection/i }));

export const Reachable: Story = {
  render: () => <Harness fetchStub={stub(async () => json(result()))} />,
  play: async () => {
    const canvas = screen();
    await press();
    await waitFor(() => expect(canvas.getByRole("status")).toBeVisible());
    // the count is what tells a reachable provider from a *useful* one
    await expect(canvas.getByRole("status")).toHaveTextContent("38 models");
    // the probed URL is always shown: a doubled /v1 is the most common cause of
    // a failure and is invisible without it
    await expect(canvas.getByText("https://api.openai.com/v1/models")).toBeVisible();
  },
};

// the case the button exists for: the row saved fine and the credential is wrong
export const RejectedCredential: Story = {
  render: () => (
    <Harness
      fetchStub={stub(async () =>
        json(
          result({
            reachable: false,
            status: 401,
            models_found: null,
            error: "401 Unauthorized: the upstream rejected the credential (resolved from: stored)",
          }),
        ),
      )}
    />
  ),
  play: async () => {
    const canvas = screen();
    await press();
    await waitFor(() =>
      expect(canvas.getByText(/rejected the credential/)).toBeVisible(),
    );
    // naming where the credential came from is what separates "wrong key" from
    // "no key configured"
    await expect(canvas.getByText(/resolved from: stored/)).toBeVisible();
  },
};

export const Unreachable: Story = {
  render: () => (
    <Harness
      fetchStub={stub(async () =>
        json(
          result({
            reachable: false,
            probed_url: "http://vllm.internal:8000/v1/models",
            status: null,
            models_found: null,
            error: "could not connect — check the host, port and TLS",
          }),
        ),
      )}
    />
  ),
  play: async () => {
    const canvas = screen();
    await press();
    await waitFor(() => expect(canvas.getByText(/could not connect/)).toBeVisible());
  },
};

// a KEK mismatch is not a provider problem, and the upstream is never contacted
export const StoredKeyUnreadable: Story = {
  render: () => (
    <Harness
      fetchStub={stub(async () =>
        json(
          result({
            reachable: false,
            probed_url: "",
            status: null,
            latency_ms: 0,
            credential: "stored (KEK unset)",
            models_found: null,
            error:
              "the stored credential for 'openai-primary' could not be read: ROLTER_KEK is unset or does not match the key it was sealed with. The upstream was not contacted.",
          }),
        ),
      )}
    />
  ),
  play: async () => {
    const canvas = screen();
    await press();
    await waitFor(() => expect(canvas.getByText(/ROLTER_KEK is unset/)).toBeVisible());
    await expect(canvas.getByText(/upstream was not contacted/)).toBeVisible();
  },
};

// the button must go busy, or an operator presses it repeatedly against a
// provider that is simply slow to answer
export const Testing: Story = {
  render: () => <Harness fetchStub={stub(() => new Promise<Response>(() => {}))} />,
  play: async () => {
    const canvas = screen();
    await press();
    await waitFor(() =>
      expect(canvas.getByRole("button", { name: /testing/i })).toBeDisabled(),
    );
  },
};
