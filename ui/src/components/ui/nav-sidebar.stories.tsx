import { Boxes, KeyRound, Play, ScrollText } from "lucide-react";
import type { Meta, StoryObj } from "@storybook/react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { NAV_MAX_WIDTH, NAV_MIN_WIDTH, NavSidebar } from "./nav-sidebar";

const meta = {
  title: "Navigation/NavSidebar",
  component: NavSidebar,
  args: {
    brand: "rolter",
    groups: [
      {
        items: [
          { key: "playground", label: "Playground", icon: <Play /> },
          { key: "models", label: "Models", icon: <Boxes /> },
          { key: "keys", label: "Keys", icon: <KeyRound /> },
          { key: "logs", label: "Logs", icon: <ScrollText /> },
        ],
      },
      {
        label: "Operate",
        items: [
          {
            key: "analytics",
            label: "Analytics",
            icon: <Boxes />,
            children: [
              { key: "usage", label: "Usage" },
              { key: "costs", label: "Costs" },
            ],
          },
        ],
      },
    ],
    activeKey: "models",
    searchable: true,
    collapsible: true,
    version: "v0.0.1",
    user: { name: "admin@rolter.dev", role: "Admin", initials: "A" },
  },
} satisfies Meta<typeof NavSidebar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Collapsed: Story = {
  args: { defaultCollapsed: true },
};

// the search box was the only control in the first fourteen tab stops with no
// visible focus indicator at all (#963). the ring is drawn as a box-shadow on
// the wrapping label, so "no indicator" is exactly `box-shadow: none` there
export const SearchFocusRing: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const search = canvas.getByRole("textbox", { name: /search/i });
    const wrapper = search.closest("label");
    await expect(wrapper).not.toBeNull();
    await expect(getComputedStyle(wrapper as HTMLElement).boxShadow).toBe("none");
    await userEvent.click(search);
    await expect(search).toHaveFocus();
    await expect(getComputedStyle(wrapper as HTMLElement).boxShadow).not.toBe("none");
  },
};

// the rail is a primary navigation control, so the splitter has to work for a
// keyboard user too: it is a focusable `separator` carrying its own width, and
// arrows move it in 16px steps within the same bounds a drag obeys (#950)
// a per-story key: the stories share one browser, and a width remembered by
// one of them must not decide where another one starts
const resizable = (name: string) => ({
  resizable: true,
  storageKey: `rolter.nav.width.story.${name}`,
});

// the rail animates its width, so the settled value is what matters
const expectWidth = (nav: HTMLElement, px: number) =>
  waitFor(() => expect(nav.getBoundingClientRect().width).toBe(px));

export const Resizable: Story = {
  args: resizable("default"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const handle = canvas.getByRole("separator", { name: /resize/i });
    await expect(handle).toHaveAttribute("aria-orientation", "vertical");
    await expect(handle).toHaveAttribute("aria-valuemin", String(NAV_MIN_WIDTH));
    await expect(handle).toHaveAttribute("aria-valuemax", String(NAV_MAX_WIDTH));
    const nav = canvasElement.querySelector("nav") as HTMLElement;
    await expectWidth(nav, 232);
  },
};

export const DraggedNarrow: Story = {
  args: resizable("narrow"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const nav = canvasElement.querySelector("nav") as HTMLElement;
    const handle = canvas.getByRole("separator", { name: /resize/i });
    handle.focus();
    // Home is the fastest route to the narrow bound; the rail must stop there
    // rather than continue toward an unusable width
    await userEvent.keyboard("{Home}");
    await expectWidth(nav, NAV_MIN_WIDTH);
    await userEvent.keyboard("{ArrowLeft}{ArrowLeft}");
    await expectWidth(nav, NAV_MIN_WIDTH);
    await expect(handle).toHaveAttribute("aria-valuenow", String(NAV_MIN_WIDTH));
    // a narrow rail still names every item: the labels truncate, they do not
    // fall out of the tree
    await expect(canvas.getByRole("button", { name: "Playground" })).toBeVisible();
  },
};

export const DraggedWide: Story = {
  args: resizable("wide"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const nav = canvasElement.querySelector("nav") as HTMLElement;
    const handle = canvas.getByRole("separator", { name: /resize/i });
    handle.focus();
    await userEvent.keyboard("{End}");
    await expectWidth(nav, NAV_MAX_WIDTH);
    await userEvent.keyboard("{ArrowRight}");
    await expectWidth(nav, NAV_MAX_WIDTH);
    // Enter returns the rail to the shipped default from either bound
    await userEvent.keyboard("{Enter}");
    await expectWidth(nav, 232);
  },
};

// collapsing wins over resizing: a 52px icon rail has no edge worth dragging,
// and leaving the splitter behind would let a keyboard user stretch a rail
// whose labels are hidden
export const CollapsedHasNoSplitter: Story = {
  args: { ...resizable("collapsed"), defaultCollapsed: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.queryByRole("separator")).toBeNull();
  },
};
