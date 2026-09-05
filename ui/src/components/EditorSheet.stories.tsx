import type { Meta, StoryObj } from "@storybook/react-vite";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { EditorSheet } from "./EditorSheet";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  expectClosesWithoutPrompting,
  expectSheetClosed,
  sheet,
  withConfirm,
} from "@/pages/story-harness";

/**
 * The shell around a caller-owned draft.
 *
 * `EditorSheet` owns the chrome and the discard guard only, so a story has to
 * bring a form and a dirty flag with it — this is the smallest thing that
 * behaves like the sheets that ship (ModelSheet, ProviderSheet,
 * ProviderGroupSheet all reduce to this).
 */
function Editor({
  seed = "openai-prod",
  saving = false,
  errorMessage,
  canSave = true,
}: {
  seed?: string;
  saving?: boolean;
  errorMessage?: string;
  canSave?: boolean;
}) {
  const [open, setOpen] = React.useState(true);
  const [name, setName] = React.useState(seed);
  const [saved, setSaved] = React.useState<string | null>(null);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Open editor
      </button>
      {saved != null && <p>Saved {saved}</p>}
      <EditorSheet
        open={open}
        onOpenChange={setOpen}
        title="Edit provider"
        subtitle="openai-prod · openai"
        dirty={name !== seed}
        errorMessage={errorMessage}
        saveLabel="Save provider"
        canSave={canSave}
        saving={saving}
        onSave={() => setSaved(name)}
      >
        <Field label="Name">
          <Input value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
      </EditorSheet>
    </>
  );
}

const meta = {
  title: "Overlays/EditorSheet",
  component: EditorSheet,
  parameters: { layout: "fullscreen" },
  // the stories drive the sheet through `Editor`, which owns the props; these
  // satisfy the required-prop contract for the docs page
  args: {
    open: true,
    onOpenChange: () => {},
    title: "Edit provider",
    subtitle: "openai-prod · openai",
    dirty: false,
    saveLabel: "Save provider",
    canSave: true,
    saving: false,
    onSave: () => {},
    children: null,
  },
} satisfies Meta<typeof EditorSheet>;

export default meta;
type Story = StoryObj<typeof meta>;

// the sheet portals onto document.body, so the story's canvas is empty
const screen = () => within(document.body);

export const Default: Story = {
  render: () => <Editor />,
  play: async () => {
    const dialog = within(sheet());
    await expect(dialog.getByText("Edit provider")).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Save provider" })).toBeEnabled();
  },
};

/** Saving: the primary action goes busy so it cannot be pressed twice. */
export const Saving: Story = {
  render: () => <Editor saving />,
  play: async () => {
    const dialog = within(sheet());
    await expect(dialog.getByRole("button", { name: "Save provider" })).toBeDisabled();
  },
};

/** Nothing valid to save yet — the button says so before the round trip does. */
export const CannotSave: Story = {
  render: () => <Editor canSave={false} />,
  play: async () => {
    const dialog = within(sheet());
    await expect(dialog.getByRole("button", { name: "Save provider" })).toBeDisabled();
  },
};

/** A rejected save keeps the draft on screen with the reason beside it. */
export const SaveFailed: Story = {
  render: () => (
    <Editor errorMessage="a provider with slug 'openai-prod' already exists in this org" />
  ),
  play: async () => {
    const dialog = within(sheet());
    await expect(dialog.getByText(/already exists in this org/)).toBeVisible();
    // the form is still there to correct, not replaced by the error
    await expect(dialog.getByLabelText("Name")).toBeVisible();
  },
};

export const Saves: Story = {
  render: () => <Editor />,
  play: async ({ canvasElement }) => {
    const dialog = within(sheet());
    await userEvent.clear(dialog.getByLabelText("Name"));
    await userEvent.type(dialog.getByLabelText("Name"), "openai-eu");
    await userEvent.click(dialog.getByRole("button", { name: "Save provider" }));
    await waitFor(() =>
      expect(within(canvasElement).getByText("Saved openai-eu")).toBeVisible(),
    );
  },
};

/**
 * An untouched draft closes without a prompt. A confirm on a form nobody
 * edited is what trains people to click through the one that matters (#868).
 */
export const ClosesCleanWithoutPrompting: Story = {
  render: () => <Editor />,
  play: async () => {
    await expectClosesWithoutPrompting();
  },
};

/** A dirty draft asks first, and "cancel" means the sheet stays open. */
export const DiscardGuardKeepsTheDraft: Story = {
  render: () => <Editor />,
  play: async () => {
    const dialog = within(sheet());
    await userEvent.type(dialog.getByLabelText("Name"), "-eu");
    await withConfirm(false, async () => {
      await userEvent.click(dialog.getByRole("button", { name: "Cancel" }));
    });
    await expect(screen().getByRole("dialog")).toBeVisible();
    await expect(dialog.getByLabelText("Name")).toHaveValue("openai-prod-eu");
  },
};

/** And "discard" closes it. Both answers, because only asserting one is half a test. */
export const DiscardGuardThrowsItAway: Story = {
  render: () => <Editor />,
  play: async () => {
    const dialog = within(sheet());
    await userEvent.type(dialog.getByLabelText("Name"), "-eu");
    await withConfirm(true, async () => {
      await userEvent.click(dialog.getByRole("button", { name: "Cancel" }));
    });
    await expectSheetClosed();
  },
};

/** The header's own close button runs the same guard as Cancel. */
export const HeaderCloseRunsTheGuard: Story = {
  render: () => <Editor />,
  play: async () => {
    const dialog = within(sheet());
    await userEvent.type(dialog.getByLabelText("Name"), "-eu");
    await withConfirm(true, async () => {
      await userEvent.click(dialog.getByRole("button", { name: /close/i }));
    });
    await expectSheetClosed();
  },
};
