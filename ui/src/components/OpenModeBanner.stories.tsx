import type { Meta, StoryObj } from "@storybook/react";
import { expect } from "storybook/test";

import { OpenModeBanner } from "./OpenModeBanner";

const meta = {
  title: "Components/OpenModeBanner",
  component: OpenModeBanner,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof OpenModeBanner>;

export default meta;
type Story = StoryObj<typeof meta>;

/** What an operator sees while the control plane has no admin token set. */
export const Open: Story = {
  args: { open: true },
  play: async ({ canvas }) => {
    const banner = await canvas.findByRole("status");
    // the variable that closes open mode has to be in the message itself:
    // "unauthenticated" without the remedy is just alarming
    await expect(banner).toHaveTextContent("ROLTER_ADMIN_TOKEN");
  },
};

/** The gated default: the banner must occupy no space at all. */
export const Gated: Story = {
  args: { open: false },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("status")).toBeNull();
  },
};
