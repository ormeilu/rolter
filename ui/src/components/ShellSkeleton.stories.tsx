import type { Meta, StoryObj } from "@storybook/react";
import { expect } from "storybook/test";

import { ShellSkeleton } from "./ShellSkeleton";

/**
 * What the dashboard shows for the one request it makes before anything else:
 * revalidating the stored session token against `/api/v1/auth/me` (#1196).
 * The alternatives are both worse — the login screen would be a lie whenever
 * the token turns out to be fine, and the live shell would fire every screen's
 * queries with a token that may already be dead.
 */
const meta = {
  title: "Feedback/ShellSkeleton",
  component: ShellSkeleton,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ShellSkeleton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Checking: Story = {
  play: async ({ canvas }) => {
    // announced, not just drawn: a placeholder that says nothing to a screen
    // reader is an empty page for the length of the request
    const status = await canvas.findByRole("status");
    await expect(status).toHaveAttribute("aria-busy", "true");
    await expect(status).toHaveAccessibleName(/checking your session/i);
  },
};
