import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Meta, StoryObj } from "@storybook/react";
import { useMemo } from "react";
import { expect, userEvent, within } from "storybook/test";

import { ScreenHeader } from "./ScreenHeader";

function HeaderStory({
  title,
  subtitle,
  pendingRefresh = false,
}: {
  title: string;
  subtitle: string;
  pendingRefresh?: boolean;
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
      <ScreenHeader title={title} subtitle={subtitle} />
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
