import type { Meta, StoryObj } from "@storybook/react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import Models from "./Models";
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
import type { EffectiveModelDto } from "@/lib/api";

const MODELS: EffectiveModelDto[] = [
  { model: "gpt-4o", strategy: "weighted", targets: 2, source: "db" },
  { model: "claude-sonnet", strategy: "least_load", targets: 1, source: "config" },
];

const loaded = routes([
  // longest first: `/models/prices` would otherwise be answered by `/models`
  ["/model-prices", () => []],
  ["/currency", () => ({ base: "USD", rates: {} })],
  ["/models", () => MODELS],
  ["/providers", () => []],
  ["/routes", () => []],
]);

const meta = {
  title: "Screens/Models",
  component: Models,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Models>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Models />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("gpt-4o")).toBeVisible());
    await expect(canvas.getByText("claude-sonnet")).toBeVisible();
  },
};

export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Models />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectSkeleton(canvasElement);
  },
};

// nothing configured at all: the CTA opens the sheet that makes the first one
export const Empty: Story = {
  render: () => (
    <Harness fetchStub={routes([["/models", () => []]])}>
      <Models />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectEmptyState(canvasElement, /No models yet/, /Add model/);
  },
};

// filters on, nothing through them: a different sentence and a different button
export const NoFilterMatch: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Models />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("gpt-4o")).toBeVisible());
    await userEvent.type(canvas.getByLabelText("Search models"), "nonexistent");
    await waitFor(() => expect(canvas.getByText(/No models match/)).toBeVisible());
    await expect(canvas.getByRole("button", { name: /Clear search/i })).toBeInTheDocument();
  },
};

export const Error_: Story = {
  name: "Error",
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "boom" } }, 500))}>
      <Models />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /failed to return models/i);
  },
};

export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "forbidden" } }, 403))}>
      <Models />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /You do not have access to models/);
  },
};
