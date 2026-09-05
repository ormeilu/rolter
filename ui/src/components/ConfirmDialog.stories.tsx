import type { Meta, StoryObj } from "@storybook/react";
import * as React from "react";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";

import { ConfirmDialog } from "./ConfirmDialog";

const meta = {
  title: "Overlays/ConfirmDialog",
  component: ConfirmDialog,
  parameters: { layout: "centered" },
  args: {
    open: true,
    title: "Delete channel ops-slack?",
    description:
      "Alerts routed to this webhook stop being delivered, and every rule pointing at it loses its destination.",
    confirmLabel: "Delete channel",
    tone: "danger",
    pending: false,
    onOpenChange: fn(),
    onConfirm: fn(),
  },
} satisfies Meta<typeof ConfirmDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

// the dialog portals onto document.body, so canvasElement is empty
const screen = () => within(document.body);

export const Default: Story = {
  play: async ({ args }) => {
    const canvas = screen();
    await waitFor(() => expect(canvas.getByRole("dialog")).toBeVisible());
    await expect(canvas.getByText("Delete channel ops-slack?")).toBeVisible();
    // no error line until something actually failed
    await expect(canvas.queryByRole("alert")).toBeNull();

    await userEvent.click(canvas.getByRole("button", { name: "Delete channel" }));
    await expect(args.onConfirm).toHaveBeenCalled();
    // confirming does not close the dialog — the caller does that on success,
    // so a failed mutation still has somewhere to report itself
    await expect(args.onOpenChange).not.toHaveBeenCalled();
  },
};

export const CancelCloses: Story = {
  play: async ({ args }) => {
    const canvas = screen();
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }));
    await expect(args.onOpenChange).toHaveBeenCalledWith(false);
    await expect(args.onConfirm).not.toHaveBeenCalled();
  },
};

export const Pending: Story = {
  args: { pending: true },
  play: async () => {
    const canvas = screen();
    // both buttons are out of reach while the request is on the wire: the
    // confirm because it would double-fire, the cancel because it cannot
    // recall a request that already left
    await expect(canvas.getByRole("button", { name: "Delete channel" })).toBeDisabled();
    await expect(canvas.getByRole("button", { name: "Cancel" })).toBeDisabled();
  },
};

export const Failed: Story = {
  args: { error: new Error("channel is referenced by 2 alert rules") },
  play: async () => {
    const canvas = screen();
    const alert = await canvas.findByRole("alert");
    // the control plane's own message, verbatim
    await expect(alert).toHaveTextContent("channel is referenced by 2 alert rules");
    // and the dialog stays usable so the operator can retry or back out
    await expect(canvas.getByRole("button", { name: "Delete channel" })).toBeEnabled();
  },
};

// a non-Error rejection (a thrown string, a rejected promise carrying a code)
// still has to reach the operator rather than render as "[object Object]"
export const FailedWithANonError: Story = {
  args: { error: "upstream timed out" },
  play: async () => {
    await expect(await screen().findByRole("alert")).toHaveTextContent("upstream timed out");
  },
};

export const NeutralTone: Story = {
  args: {
    tone: "default",
    title: "Rotate this key?",
    description: "The current secret stops working the moment the new one is issued.",
    confirmLabel: "Rotate key",
  },
  render: (args) => {
    // the neutral tone exists for actions that are irreversible without being
    // deletions; rotate is the one that motivated it
    const [open, setOpen] = React.useState(true);
    return <ConfirmDialog {...args} open={open} onOpenChange={setOpen} />;
  },
};
