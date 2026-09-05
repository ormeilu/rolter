import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import Logs from "./Logs";
import {
  Harness,
  expectEmptyState,
  expectLoadError,
  expectSkeleton,
  json,
  pending,
  routes,
  scoped,
  type FetchStub,
} from "./story-harness";
import type { InvocationRow, ModelPriceRow } from "@/lib/api";
import { formattersFor } from "@/lib/i18n/format";
import { atMobile, atTablet, expectNoHorizontalOverflow } from "@/lib/story-viewport";

// the formatter the screen itself uses, so a story asserts the house format
// rather than a second copy of it
const fmt = formattersFor("en");

const PRICED_AT = "2026-10-05T12:34:56.789Z";

const row = (over: Partial<InvocationRow>): InvocationRow => ({
  ts: PRICED_AT,
  request_id: "req-1",
  trace_id: "trace-1",
  org_id: "org-1",
  team_id: "team-1",
  project_id: "project-1",
  virtual_key_id: "vk-1",
  model: "gpt-4o",
  provider: "openai",
  target: "openai/gpt-4o",
  variant: "",
  status: 200,
  stream: 0,
  cache_hit: 0,
  cache_read_tokens: 0,
  cache_write_tokens: 0,
  prompt_tokens: 8000,
  completion_tokens: 4345,
  total_tokens: 12345,
  cost_usd: 0.0123,
  latency_ms: 842,
  ttft_ms: 120,
  error: "",
  ...over,
});

const ROWS: InvocationRow[] = [
  row({}),
  // served against a model with no price row: the zero is the absence of a
  // cost, not a cost of zero (#969)
  row({ request_id: "req-2", model: "internal-llama", provider: "vllm", cost_usd: 0 }),
];

const PRICES: ModelPriceRow[] = [
  {
    id: "price-1",
    model: "gpt-4o",
    input_per_mtok: "2.50",
    output_per_mtok: "10.00",
    currency: "USD",
    created_at: "2026-01-01T00:00:00Z",
  },
];

const withLogs = (rows: InvocationRow[], base = "USD"): FetchStub =>
  routes([
    ["/api/v1/analytics/invocations", () => ({ data: rows })],
    ["/api/v1/model-prices", () => PRICES],
    ["/api/v1/currency", () => ({ base, codes: [base], rates: {} })],
    ["/api/v1/models", () => []],
  ]);

const meta = {
  title: "Screens/Logs",
  component: Logs,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Logs>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={withLogs(ROWS)}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // one house stamp for the timestamp column, milliseconds included, and one
    // grouped number for tokens — neither follows the browser locale (#1182)
    await expect(await canvas.findAllByText(fmt.dateTimeMs(PRICED_AT))).toHaveLength(2);
    await expect(await canvas.findAllByText(fmt.number(12345))).toHaveLength(2);
    await expect(await canvas.findByText(fmt.currency(0.0123, "USD"))).toBeInTheDocument();
  },
};

/**
 * #969/#1182: a request against a model with no price used to read `$0.0000`,
 * which claims the request was free. The cell says "unknown" instead.
 */
export const UnpricedRequestsAreNotFree: Story = {
  render: () => (
    <Harness fetchStub={withLogs(ROWS)}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const marker = await canvas.findByTitle(/no price is configured/i);
    await expect(marker).toBeInTheDocument();
    // and the false zero is nowhere on the screen
    await expect(canvas.queryByText(fmt.currency(0, "USD"))).toBeNull();
  },
};

/** Spend is denominated in the deployment's settlement currency, not in dollars. */
export const AmountsFollowTheDeploymentCurrency: Story = {
  render: () => (
    <Harness fetchStub={withLogs([row({})], "EUR")}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText(fmt.currency(0.0123, "EUR"))).toBeInTheDocument();
    await expect(canvas.queryByText(fmt.currency(0.0123, "USD"))).toBeNull();
  },
};

export const Empty: Story = {
  render: () => (
    <Harness fetchStub={withLogs([])}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    // no filter is set, so this is "the gateway has served nothing yet" rather
    // than "your filters excluded everything" (#1180)
    await expectEmptyState(canvasElement, /Nothing logged yet/);
  },
};

// the screen had no loading indicator at all: a slow ClickHouse read was
// indistinguishable from a deployment that had served nothing (#1180)
export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectSkeleton(canvasElement);
  },
};

export const Failed: Story = {
  render: () => (
    <Harness
      fetchStub={scoped(async () => json({ error: { message: "clickhouse refused" } }, 500))}
    >
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /failed to return request logs/i);
  },
};

// a 403 is not a 500: retrying cannot fix a permission, so no retry is offered
export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={scoped(async () => json({ error: { message: "forbidden" } }, 403))}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectLoadError(canvasElement, /You do not have access to request logs/);
  },
};

/**
 * #1203: the 248px filter rail and the 380px detail drawer both sat in the
 * flow, so at 375px the table had 127px and the page scrolled sideways. Both
 * are overlays at this width, and the table scrolls inside its own container.
 */
export const Mobile: Story = {
  ...atMobile,
  render: () => (
    <Harness fetchStub={withLogs(ROWS)}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findAllByText(fmt.dateTimeMs(PRICED_AT));
    await expectNoHorizontalOverflow();
  },
};

export const Tablet: Story = {
  ...atTablet,
  render: () => (
    <Harness fetchStub={withLogs(ROWS)}>
      <Logs />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findAllByText(fmt.dateTimeMs(PRICED_AT));
    await expectNoHorizontalOverflow();
  },
};
