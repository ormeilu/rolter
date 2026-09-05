import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";

import Config from "./Config";
import { Harness, json, pending, scoped } from "./story-harness";

// a slice of what GET /api/v1/config returns: the three tabled sections plus
// a handful of the ~40 generic ones the screen renders collapsed (#1204)
const CONFIG = {
  providers: [{ name: "openai-prod", kind: "openai", api_base: "https://api.openai.com/v1" }],
  routes: [{ model: "gpt-4o", strategy: "round_robin", targets: [{ provider: "openai-prod", weight: 1 }] }],
  virtual_keys: [],
  db_virtual_keys: [{ key_hash: "", id: "k1" }],
  mcp_oauth_sessions: [],
  server: { host: "0.0.0.0", port: 4000, workers: 4 },
  cache: { enabled: true, ttl_secs: 300 },
  budgets: [],
  unpriced_policy: "ignore",
};

const loaded = scoped(async (input) =>
  String(input).includes("/api/v1/config") ? json(CONFIG) : json([]),
);
const forbidden = scoped(async () => json({ error: { message: "forbidden" } }, 403));

const meta = {
  title: "Screens/Config",
  component: Config,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Config>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Config />
    </Harness>
  ),
  // every non-tabled section is listed once, collapsed, with its shape; the
  // gateway-only sections (digests, redacted sessions) are not
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText("4 sections")).toBeVisible();
    await expect(canvas.getByText("server")).toBeVisible();
    await expect(canvas.getByText("3 fields")).toBeVisible();
    await expect(canvas.queryByText("db_virtual_keys")).toBeNull();
    await expect(canvas.queryByText("mcp_oauth_sessions")).toBeNull();

    await userEvent.click(canvas.getByText("cache"));
    await expect(canvas.getByText(/"ttl_secs": 300/)).toBeVisible();
  },
};

export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Config />
    </Harness>
  ),
};

export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={forbidden}>
      <Config />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("alert")).toHaveTextContent(/do not have access/i);
  },
};
