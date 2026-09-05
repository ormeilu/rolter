import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import { useMemo } from "react";
import { expect, userEvent, within } from "storybook/test";

import { ScreenHeader } from "./ScreenHeader";
import { atMobile, expectNoHorizontalOverflow } from "@/lib/story-viewport";

function HeaderStory({
  title,
  subtitle,
  pendingRefresh = false,
  onOpenNav,
}: {
  title: string;
  subtitle: string;
  pendingRefresh?: boolean;
  onOpenNav?: () => void;
}) {
  const queryClient = useMemo(() => {
    const client = new QueryClient();
    if (pendingRefresh) {
      client.invalidateQueries = () => new Promise<void>(() => {});
    }
    return client;
  }, [pendingRefresh]);

  return (
    <QueryClientProvider client={queryClient}>
      <ScreenHeader title={title} subtitle={subtitle} onOpenNav={onOpenNav} />
    </QueryClientProvider>
  );
}

const meta = {
  title: "Components/ScreenHeader",
  component: ScreenHeader,
  parameters: { layout: "fullscreen" },
  args: { title: "Models", subtitle: "Manage routed models" },
} satisfies Meta<typeof ScreenHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => <HeaderStory {...args} />,
};

export const Refreshing: Story = {
  render: (args) => <HeaderStory {...args} pendingRefresh />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const refresh = canvas.getByRole("button", { name: "Refresh data" });

    await userEvent.click(refresh);

    const busyRefresh = canvas.getByRole("button", { name: "Refreshing data" });
    await expect(busyRefresh).toBeDisabled();
    await expect(busyRefresh).toHaveAttribute("aria-busy", "true");
    await expect(busyRefresh.querySelector("svg")).toHaveClass("motion-safe:animate-spin");
  },
};

/**
 * Below `md` the header carries the only way into the navigation, and the
 * status pill takes a row of its own rather than landing on the subtitle
 * (#959). The longest subtitle in the catalogs is the one that used to wrap to
 * one word per line.
 */
export const Mobile: Story = {
  ...atMobile,
  args: {
    title: "Dashboard",
    subtitle: "Live overview of traffic, spend, latency, and provider health.",
  },
  render: (args) => <HeaderStory {...args} onOpenNav={() => {}} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const menu = canvas.getByRole("button", { name: "Open navigation" });
    // a touch target, not a 24px pointer one
    await expect(menu.getBoundingClientRect().height).toBeGreaterThanOrEqual(40);
    // the pill and the title do not share a line, so neither sits on the other
    const pill = canvas.getByText("gateway healthy");
    const heading = canvas.getByRole("heading", { name: "Dashboard" });
    await expect(pill.getBoundingClientRect().top).toBeGreaterThanOrEqual(
      heading.getBoundingClientRect().bottom,
    );
    await expectNoHorizontalOverflow();
  },
};
