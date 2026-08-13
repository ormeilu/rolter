import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import Pricing from "./Pricing";
import { Harness, clickWhenEnabled, pending, routes, sheet } from "./story-harness";
import type { CurrencySettings, ModelPriceRow } from "@/lib/api";

const price = (
  model: string,
  currency: string,
  id = model,
): ModelPriceRow => ({
  id,
  model,
  input_per_mtok: "2.50",
  output_per_mtok: "10.00",
  cached_input_per_mtok: "1.25",
  currency,
  created_at: "2026-07-01T00:00:00Z",
});

const PRICES: ModelPriceRow[] = [
  price("gpt-4o", "USD"),
  price("mistral-large", "EUR"),
  // priced in a code this deployment's rate table carries — #965's whole point
  price("yandex-gpt", "RUB"),
];

/** a deployment that settles in USD and has configured a RUB rate */
const CONFIGURED: CurrencySettings = {
  base: "USD",
  codes: ["USD", "EUR", "RUB"],
  rates: { USD: 1, EUR: 1.09, RUB: 0.011 },
};

/** the same prices against a table that has since dropped EUR and RUB */
const NARROWED: CurrencySettings = {
  base: "USD",
  codes: ["USD"],
  rates: { USD: 1 },
};

const withCurrency = (settings: CurrencySettings, prices = PRICES) =>
  routes([
    ["/api/v1/currency", () => settings],
    ["/model-prices", () => prices],
  ]);

const meta = {
  title: "Screens/Pricing",
  component: Pricing,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Pricing>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={withCurrency(CONFIGURED)}>
      <Pricing />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // every price converts, so nothing is flagged
    await expect(await canvas.findByText("gpt-4o")).toBeInTheDocument();
    await expect(canvas.queryByText(/no conversion rate/i)).not.toBeInTheDocument();
  },
};

export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Pricing />
    </Harness>
  ),
};

export const Empty: Story = {
  render: () => (
    <Harness fetchStub={withCurrency(CONFIGURED, [])}>
      <Pricing />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText(/no model prices set yet/i)).toBeInTheDocument();
  },
};

/**
 * #965: a price stored in a code the rate table no longer carries.
 *
 * It must still display — dropping it would hide real pricing — but its spend
 * is missing from base-currency totals, so it is flagged rather than silently
 * read as USD.
 */
export const UnconvertibleCurrency: Story = {
  render: () => (
    <Harness fetchStub={withCurrency(NARROWED)}>
      <Pricing />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // the EUR and RUB prices are still listed...
    await expect(await canvas.findByText("mistral-large")).toBeInTheDocument();
    await expect(canvas.getByText("yandex-gpt")).toBeInTheDocument();
    // ...and both are called out as unconvertible, while the USD one is not
    await expect(await canvas.findAllByText(/no conversion rate/i)).toHaveLength(2);
  },
};

/**
 * The currency field is a free-text input rather than a chooser on this screen:
 * the catalog is global and an operator may be pricing in a code before adding
 * its rate. The control plane rejects the write, which is where the guard is.
 */
export const AddsAPrice: Story = {
  render: () => (
    <Harness fetchStub={withCurrency(CONFIGURED)}>
      <Pricing />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /add price/i);
    await expect(within(sheet()).getByLabelText("Currency")).toHaveValue("USD");
  },
};
