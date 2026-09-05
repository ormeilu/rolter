import type { Meta, StoryObj } from "@storybook/react";
import { expect, within } from "storybook/test";

import { EmptyState } from "./empty-state";
import { Table, type TableColumn } from "./table";

interface Row extends Record<string, unknown> {
  id: string;
  model: string;
  provider: string;
  requests: number;
  p95: string;
}

const ROWS: Row[] = [
  { id: "r1", model: "gpt-4o", provider: "openai-prod", requests: 18422, p95: "812 ms" },
  { id: "r2", model: "claude-sonnet", provider: "anthropic-prod", requests: 9310, p95: "1.2 s" },
  { id: "r3", model: "llama-3.1-70b", provider: "vllm-cluster", requests: 4180, p95: "340 ms" },
];

const COLUMNS: TableColumn<Row>[] = [
  { key: "model", header: "Model", mono: true },
  { key: "provider", header: "Provider" },
  { key: "requests", header: "Requests", align: "right" },
  { key: "p95", header: "p95", align: "right", mono: true },
];

const meta = {
  title: "Display/Table",
  component: Table,
  parameters: { layout: "padded" },
} satisfies Meta<typeof Table>;

export default meta;

// `Table` is generic in its row type, so `StoryObj<typeof meta>` would type
// `args` against the `Record<string, unknown>` constraint rather than against
// `Row` and reject the fixture below. Every story renders its own table, so the
// story type only has to allow a bare `render`.
type Story = StoryObj;

export const Default: Story = {
  render: () => <Table columns={COLUMNS} data={ROWS} rowKey="id" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // header cells are real column headers, not styled divs: that is what lets
    // a screen reader say which column a value is in
    await expect(canvas.getByRole("columnheader", { name: "Model" })).toBeVisible();
    await expect(canvas.getAllByRole("row")).toHaveLength(ROWS.length + 1);
    await expect(canvas.getByText("llama-3.1-70b")).toBeVisible();
  },
};

/**
 * A `render` per column, for cells that are more than the raw value — this is
 * how the dashboard puts badges, links and copy buttons inside a table without
 * every screen re-deriving the markup.
 */
export const RenderedCells: Story = {
  render: () => (
    <Table
      columns={[
        ...COLUMNS.slice(0, 2),
        {
          key: "requests",
          header: "Requests",
          align: "right",
          render: (value) => (
            <span className="font-mono text-xs">{(value as number).toLocaleString("en-US")}</span>
          ),
        },
      ]}
      data={ROWS}
      rowKey="id"
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("18,422")).toBeVisible();
  },
};

/**
 * Loaded and empty (#1180). The placeholder is rendered in one full-width cell
 * so it sits inside the table's border with the column headers still above it:
 * a header row over nothing reads as a screen that is still loading.
 */
export const Empty: Story = {
  render: () => (
    <Table
      columns={COLUMNS}
      data={[]}
      empty={
        <EmptyState
          title="No traffic in this window"
          description="Widen the time range, or send a request through the gateway to see it here."
        />
      }
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("No traffic in this window")).toBeVisible();
    // the columns survive the empty state — they say what a row would carry
    await expect(canvas.getByRole("columnheader", { name: "Provider" })).toBeVisible();
  },
};

/**
 * With no `empty` prop and no rows the table is a bare header, which is the
 * shape #1180 was filed against. Kept as a story so the difference from
 * `Empty` above is visible side by side rather than argued about.
 */
export const EmptyWithoutPlaceholder: Story = {
  render: () => <Table columns={COLUMNS} data={[]} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByRole("row")).toHaveLength(1);
  },
};

/**
 * The scroll container is focusable: a table wider than its panel would
 * otherwise hide its right-hand columns from anyone not using a mouse (#1181).
 */
export const ScrollsFromTheKeyboard: Story = {
  render: () => (
    <div className="max-w-[320px]">
      <Table columns={COLUMNS} data={ROWS} rowKey="id" />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const scroller = canvasElement.querySelector<HTMLElement>("[tabindex='0']");
    await expect(scroller).not.toBeNull();
    scroller?.focus();
    await expect(scroller).toHaveFocus();
  },
};
