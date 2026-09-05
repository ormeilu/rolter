import type { Meta, StoryObj } from "@storybook/react";
import { expect, waitFor, within } from "storybook/test";

import Rbac from "./Rbac";
import {
  Harness,
  expectLoadError,
  expectSkeleton,
  json,
  pending,
  routes,
  scoped,
} from "./story-harness";

const MEMBERSHIPS = [
  { team_id: "team-1", user_id: "u-1", role: "admin" },
  { team_id: "team-1", user_id: "u-2", role: "member" },
  // the same user twice: the count is distinct users, not grants
  { team_id: "team-2", user_id: "u-2", role: "member" },
];

const meta = {
  title: "Screens/Rbac",
  component: Rbac,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Rbac>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={routes([["/memberships", () => MEMBERSHIPS]])}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // the count is distinct users, not grants: u-2 holds member in two teams
    // and still counts once
    await waitFor(() => expect(canvas.getAllByText("1 member")).toHaveLength(2));
    // and a role nobody holds says zero rather than going blank
    await expect(canvas.getByText("0 members")).toBeVisible();
  },
};

/**
 * The screen had no loading indicator at all (#1180). The matrix itself is the
 * control API's static contract and renders instantly, so the only thing in
 * flight is the per-role member count — and that is the only thing a skeleton
 * stands in for. Blanking the matrix would be a regression, not a fix.
 */
export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expectSkeleton(canvasElement);
    // the contract is still readable while the counts load
    await expect(canvas.getByText("Resource")).toBeVisible();
  },
};

// a failed membership read must not take the matrix off screen with it: the
// banner sits above, and the counts say they are unknown rather than zero
export const Error_: Story = {
  name: "Error",
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "boom" } }, 500))}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expectLoadError(canvasElement, /failed to return role membership counts/i);
    await expect(canvas.getAllByText("members unknown").length).toBeGreaterThan(0);
    await expect(canvas.getByText("Resource")).toBeVisible();
  },
};

export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "forbidden" } }, 403))}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /You do not have access to role membership counts/);
  },
};
