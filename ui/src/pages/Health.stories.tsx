import type { Meta, StoryObj } from "@storybook/react";
import { expect, waitFor, within } from "storybook/test";

import Health from "./Health";
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
import type { MttrRow, TimelineRow, UptimeRow } from "@/lib/api";

const UPTIME: UptimeRow[] = [
  {
    provider: "openai",
    target_id: "gpt-4o@primary",
    events: 4210,
    ok: 4198,
    errors: 9,
    timeouts: 3,
    uptime: 0.9971,
    failure_rate: 0.0029,
    error_budget_burn: 0.29,
    sla_breached: 0,
    last_event: "2026-08-06T10:00:00Z",
  },
  // past its error budget: the card paints danger and the breaker reads tripped
  {
    provider: "anthropic",
    target_id: "sonnet@eu",
    events: 980,
    ok: 902,
    errors: 71,
    timeouts: 7,
    uptime: 0.9204,
    failure_rate: 0.0796,
    error_budget_burn: 7.96,
    sla_breached: 1,
    last_event: "2026-08-06T09:58:00Z",
  },
];

const MTTR: MttrRow[] = [
  { provider: "openai", target_id: "gpt-4o@primary", mttr_seconds: 42, incidents: 2 },
  { provider: "anthropic", target_id: "sonnet@eu", mttr_seconds: 913, incidents: 5 },
];

const TIMELINE: TimelineRow[] = Array.from({ length: 12 }, (_, i) => ({
  bucket: `2026-08-06T${String(i).padStart(2, "0")}:00:00Z`,
  provider: "openai",
  target_id: "gpt-4o@primary",
  events: 300,
  ok: i === 7 ? 280 : 300,
  errors: i === 7 ? 20 : 0,
  timeouts: 0,
}));

const loaded = routes([
  ["/health/uptime", () => ({ data: UPTIME })],
  ["/health/mttr", () => ({ data: MTTR })],
  ["/health/timeline", () => ({ data: TIMELINE })],
]);

const meta = {
  title: "Screens/Health",
  component: Health,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Health>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Health />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("sonnet@eu")).toBeVisible());
    // the breached target is the one the operator has to act on, so it says so
    await expect(canvas.getByText("tripped")).toBeVisible();
  },
};

// three queries, none of them settled: the card grid stands in for itself so
// the layout does not jump when the rollups land
export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Health />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectSkeleton(canvasElement);
  },
};

// a deployment that has served no traffic has no rollups to compute — that is
// not a failure, and the CTA points at the one thing that produces an event
export const Empty: Story = {
  render: () => (
    <Harness fetchStub={routes([["/health/", () => ({ data: [] })]])}>
      <Health />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectEmptyState(canvasElement, /No health events recorded yet/);
  },
};

export const Error_: Story = {
  name: "Error",
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "boom" } }, 500))}>
      <Health />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /failed to return health rollups/i);
  },
};

// retrying a 403 cannot work, so LoadError withholds the button and says who can
export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "forbidden" } }, 403))}>
      <Health />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /You do not have access to health rollups/);
  },
};
