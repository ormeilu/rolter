import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { Button } from "./button";
import { Toaster } from "./toaster";
import { ToastProvider, useToast } from "@/lib/toast";

const meta = {
  title: "Feedback/Toaster",
  component: Toaster,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Toaster>;

export default meta;
type Story = StoryObj<typeof meta>;

function Demo() {
  const toast = useToast();
  return (
    <div className="flex gap-2 p-6">
      <Button onClick={() => toast.push({ tone: "success", title: "Provider saved" })}>
        Success
      </Button>
      <Button
        variant="outline"
        onClick={() =>
          toast.push({
            tone: "error",
            title: "Could not delete openai-prod",
            detail: "provider is still the target of 3 routes",
          })
        }
      >
        Error
      </Button>
      <Button
        variant="ghost"
        onClick={() => toast.push({ tone: "info", title: "Snapshot version 42 applied" })}
      >
        Info
      </Button>
    </div>
  );
}

const Stage = () => (
  <ToastProvider>
    <Demo />
    <Toaster />
  </ToastProvider>
);

export const Default: Story = { render: () => <Stage /> };

// a success is announced politely, a failure assertively, and either can be
// dismissed by hand before its timer runs out
export const AnnouncesAndDismisses: Story = {
  render: () => <Stage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "Success" }));
    const status = canvas.getByRole("status");
    // the card fades in, so visibility is awaited rather than asserted at once
    await waitFor(() => expect(within(status).getByText("Provider saved")).toBeVisible());

    await userEvent.click(canvas.getByRole("button", { name: "Error" }));
    const alert = canvas.getByRole("alert");
    await waitFor(() => expect(within(alert).getByText(/could not delete/i)).toBeVisible());
    await expect(within(alert).getByText(/still the target/i)).toBeInTheDocument();

    await userEvent.click(within(alert).getByRole("button", { name: "Dismiss notification" }));
    await waitFor(() => expect(within(alert).queryByText(/could not delete/i)).toBeNull());
    // the success is still on its own timer
    await expect(within(status).getByText("Provider saved")).toBeInTheDocument();
  },
};
