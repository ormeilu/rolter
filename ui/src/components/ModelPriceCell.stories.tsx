import type { Meta, StoryObj } from "@storybook/react";
import { expect } from "storybook/test";

import { ModelPriceCell } from "./ModelPriceCell";

const meta = {
  title: "Components/ModelPriceCell",
  component: ModelPriceCell,
} satisfies Meta<typeof ModelPriceCell>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Priced: Story = {
  args: { priced: true, inPrice: "$0.15", outPrice: "$0.60" },
  play: async ({ canvas }) => {
    await expect(await canvas.findByText(/\$0\.15 · \$0\.60/)).toBeInTheDocument();
    await expect(canvas.queryByText(/no price set/i)).toBeNull();
  },
};

/** The #969 case: served, recorded, but not accounted for. */
export const Unpriced: Story = {
  args: { priced: false, inPrice: "—", outPrice: "—" },
  play: async ({ canvas }) => {
    const badge = await canvas.findByText(/no price set/i);
    await expect(badge).toBeInTheDocument();
    // the dash must not be what says it: "nothing to show" and "we cannot
    // account for this traffic" are different claims
    await expect(canvas.queryByText(/— · —/)).toBeNull();
    await expect(badge).toHaveAttribute("title", expect.stringContaining("unpriced"));
  },
};

/**
 * The price catalogue could not be read. Not knowing whether a model is priced
 * is not the same as knowing it is unpriced, so the cell stays quiet.
 */
export const CatalogueUnavailable: Story = {
  args: { priced: null, inPrice: "—", outPrice: "—" },
  play: async ({ canvas }) => {
    await expect(canvas.queryByText(/no price set/i)).toBeNull();
    await expect(await canvas.findByText(/— · —/)).toBeInTheDocument();
  },
};
