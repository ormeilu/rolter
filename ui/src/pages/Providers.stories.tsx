import type { Meta, StoryObj } from "@storybook/react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import Providers from "./Providers";
import {
  Harness,
  expectEmptyState,
  expectLoadError,
  expectSkeleton,
  json,
  pending,
  routes,
  scoped,
} from "./story-harness";
import type { ProviderRow } from "@/lib/api";
import { atMobile, atTablet, expectNoHorizontalOverflow } from "@/lib/story-viewport";

const PROVIDERS: ProviderRow[] = [
  {
    id: "p-1",
    org_id: "org-1",
    name: "openai-prod",
    slug: "openai-prod",
    kind: "openai",
    api_base: "https://api.openai.com/v1",
    api_key_env: "OPENAI_API_KEY",
    created_at: "2026-01-02T00:00:00Z",
  },
  {
    id: "p-2",
    org_id: "org-1",
    name: "anthropic-eu",
    slug: "anthropic-eu",
    kind: "anthropic",
    api_base: "https://api.anthropic.com",
    api_key_env: "ANTHROPIC_API_KEY",
    created_at: "2026-01-09T00:00:00Z",
  },
];

const loaded = routes([
  ["/providers", () => PROVIDERS],
  ["/config/problems", () => []],
]);

const meta = {
  title: "Screens/Providers",
  component: Providers,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Providers>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getAllByText("openai-prod").length).toBeGreaterThan(0));
    await expect(canvas.getAllByText("anthropic-eu").length).toBeGreaterThan(0);
  },
};

// the rows are skeletons inside the real table, so the column headers stay put
// and the list does not jump a row-height when the data lands
export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectSkeleton(canvasElement);
  },
};

// a fresh install: the CTA is the whole point of the screen
export const Empty: Story = {
  render: () => (
    <Harness fetchStub={routes([["/providers", () => []]])}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectEmptyState(canvasElement, /No providers yet/, /Add provider/);
  },
};

// a search that matched nothing is not the same answer as an empty org: the
// copy blames the query and offers to clear it rather than to create a row
export const NoSearchMatch: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getAllByText("openai-prod").length).toBeGreaterThan(0));
    await userEvent.type(canvas.getByLabelText("Search providers"), "cohere");
    await waitFor(() => expect(canvas.getByText(/No providers match/)).toBeVisible());
    await expect(canvas.getByRole("button", { name: /Clear search/i })).toBeInTheDocument();
  },
};

export const Error_: Story = {
  name: "Error",
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "boom" } }, 500))}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /failed to return providers/i);
  },
};

export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "forbidden" } }, 403))}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /You do not have access to providers/);
  },
};

/**
 * The provider list is six columns wide and stays six columns wide: below `md`
 * it scrolls inside its own border instead of dragging the page sideways under
 * the shell (#1203).
 */
export const Mobile: Story = {
  ...atMobile,
  render: () => (
    <Harness fetchStub={loaded}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getAllByText("openai-prod").length).toBeGreaterThan(0));
    await expectNoHorizontalOverflow();
  },
};

export const Tablet: Story = {
  ...atTablet,
  render: () => (
    <Harness fetchStub={loaded}>
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getAllByText("openai-prod").length).toBeGreaterThan(0));
    await expectNoHorizontalOverflow();
  },
};
