import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react-vite";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import GuardrailProviders from "./GuardrailProviders";
import { cancelConfirmation, confirmDestructive, recording } from "./story-harness";
import type { GuardrailProviderRow } from "@/lib/api";

const PROVIDERS: GuardrailProviderRow[] = [
  {
    id: "provider-primary",
    name: "Production LLM Guard",
    enabled: true,
    url: "https://guardrails.internal/v1/evaluate",
    stage: "pre_call",
    timeout_ms: 1800,
    max_retries: 1,
    failure_mode: "fail_closed",
    max_body_bytes: 65536,
    auth_kind: "bearer",
    auth_env: "ROLTER_GUARDRAIL_TOKEN",
    created_at: "2026-08-02T00:00:00Z",
    updated_at: "2026-08-02T00:00:00Z",
  },
  {
    id: "provider-fallback",
    name: "Staging evaluator",
    enabled: false,
    url: "https://guardrails.staging.internal/evaluate",
    stage: "pre_call",
    timeout_ms: 2500,
    max_retries: 0,
    failure_mode: "fail_open",
    max_body_bytes: 32768,
    auth_kind: "none",
    auth_env: null,
    created_at: "2026-08-02T00:00:00Z",
    updated_at: "2026-08-02T00:00:00Z",
  },
];

type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
function Harness({ fetchStub }: { fetchStub: FetchStub }) {
  const original = React.useRef<typeof globalThis.fetch | null>(null);
  const client = React.useMemo(() => {
    original.current ??= globalThis.fetch;
    globalThis.fetch = fetchStub as typeof globalThis.fetch;
    return new QueryClient({ defaultOptions: { queries: { retry: false } } });
  }, [fetchStub]);
  React.useEffect(() => () => { if (original.current) globalThis.fetch = original.current; }, []);
  return <QueryClientProvider client={client}><GuardrailProviders /></QueryClientProvider>;
}

const meta = { title: "Screens/GuardrailProviders", component: GuardrailProviders, parameters: { layout: "fullscreen" } } satisfies Meta<typeof GuardrailProviders>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = { render: () => <Harness fetchStub={async () => json(PROVIDERS)} /> };
export const Loading: Story = { render: () => <Harness fetchStub={() => new Promise<Response>(() => {})} /> };
export const Empty: Story = { render: () => <Harness fetchStub={async () => json([])} /> };
export const Error: Story = { render: () => <Harness fetchStub={async () => json({ error: { message: "registry offline" } }, 503)} /> };

export const RegistersProvider: Story = {
  render: () => <Harness fetchStub={async (_input, init) => init?.method === "POST" ? json(PROVIDERS[0], 201) : json(PROVIDERS)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: /add provider/i }));
    const dialog = within(document.body).getByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Provider name"), "Policy service");
    await expect(within(dialog).getByRole("button", { name: "Save provider" })).toBeEnabled();
  },
};

// deleting an enabled provider stops external enforcement outright, which is
// too large a change for a bare window.confirm to carry (#1179)
const deletes = recording(async (_input, init) =>
  init?.method === "DELETE" ? json({}, 204) : json(PROVIDERS),
);

export const ConfirmsBeforeDeletingAProvider: Story = {
  render: () => <Harness fetchStub={deletes.stub} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const del = async () => (await canvas.findAllByRole("button", { name: "Delete" }))[0];

    await userEvent.click(await del());
    await cancelConfirmation();
    deletes.expectNotSent("DELETE", "/guardrails/providers/provider-primary");

    await userEvent.click(await del());
    await confirmDestructive(/Production LLM Guard/, "Delete");
    await deletes.expectSent("DELETE", "/guardrails/providers/provider-primary");
  },
};

export const ExplainsFailOpen: Story = {
  render: () => <Harness fetchStub={async () => json(PROVIDERS)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click((await canvas.findAllByRole("button", { name: "Edit provider" }))[1]);
    await waitFor(() => expect(within(document.body).getByText(/fail-open favors availability/i)).toBeVisible());
  },
};
