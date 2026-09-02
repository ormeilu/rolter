import { Boxes, KeyRound, Play, ScrollText } from "lucide-react";
import type { Meta, StoryObj } from "@storybook/react";
import { expect, userEvent, within } from "storybook/test";

import { NavSidebar } from "./nav-sidebar";

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
