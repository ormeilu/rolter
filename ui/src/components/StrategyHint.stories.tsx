import type { Meta, StoryObj } from "@storybook/react";
import { expect } from "storybook/test";

import { StrategyHint } from "./StrategyHint";

const meta = {
  title: "Components/StrategyHint",
  component: StrategyHint,
} satisfies Meta<typeof StrategyHint>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Degrades to least-load without KV events / an LMCache controller. */
export const NeedsTelemetry: Story = {
  args: { strategy: "precise_cache_aware" },
  play: async ({ canvas }) => {
    const note = await canvas.findByRole("note");
    await expect(note).toHaveTextContent(/least-load/);
  },
};

export const LmCacheNeedsTelemetry: Story = {
  args: { strategy: "lmcache_aware" },
  play: async ({ canvas }) => {
    await expect(await canvas.findByRole("note")).toBeInTheDocument();
  },
};

/** Governed by the deployment-wide policy, shown only because it is the value. */
export const DeploymentWide: Story = {
  args: { strategy: "adaptive" },
  play: async ({ canvas }) => {
    await expect(await canvas.findByRole("note")).toHaveTextContent(/adaptive-routing policy/);
  },
};

/** Most strategies need no caveat, and a hint on every one would be noise. */
export const NoCaveat: Story = {
  args: { strategy: "round_robin" },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("note")).toBeNull();
  },
};

/** Newly selectable in #897, and deliberately uncaveated: pure config. */
export const NewlySelectableNeedsNoCaveat: Story = {
  args: { strategy: "cheapest" },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("note")).toBeNull();
  },
};
