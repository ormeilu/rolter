import type { Meta, StoryObj } from "@storybook/react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { CopyAsCodeButton } from "./CodeSnippetDialog";

const meta = {
  title: "Overlays/CodeSnippetDialog",
  component: CopyAsCodeButton,
  parameters: { layout: "centered" },
  args: { request: { model: "llama-3.1-8b", prompt: "hello there" } },
} satisfies Meta<typeof CopyAsCodeButton>;

export default meta;
type Story = StoryObj<typeof meta>;

// the dialog renders through a portal onto document.body, so canvasElement is
// empty — query the whole document
const screen = () => within(document.body);

const open = async () =>
  userEvent.click(await screen().findByRole("button", { name: /copy as code/i }));

export const Curl: Story = {
  play: async () => {
    await open();
    const canvas = screen();
    await waitFor(() => expect(canvas.getByRole("dialog")).toBeVisible());
    // curl is the default because it needs no project to try
    await expect(canvas.getByText(/curl .*\/gw\/v1\/chat\/completions/)).toBeVisible();
    await expect(canvas.getByText(/llama-3\.1-8b/)).toBeVisible();
  },
};

export const SwitchesLanguage: Story = {
  play: async () => {
    await open();
    const canvas = screen();
    await waitFor(() => expect(canvas.getByRole("dialog")).toBeVisible());

    await userEvent.selectOptions(canvas.getByRole("combobox"), "python");
    await waitFor(() => expect(canvas.getByText(/from openai import OpenAI/)).toBeVisible());

    await userEvent.selectOptions(canvas.getByRole("combobox"), "javascript");
    await waitFor(() =>
      expect(canvas.getByText(/import OpenAI from "openai"/)).toBeVisible(),
    );
  },
};

// the snippet is pasted into tickets and chat, so it must never carry the
// operator's live virtual key
export const NeverInlinesTheKey: Story = {
  play: async () => {
    await open();
    const canvas = screen();
    await waitFor(() => expect(canvas.getByRole("dialog")).toBeVisible());
    for (const lang of ["curl", "python", "javascript"]) {
      await userEvent.selectOptions(canvas.getByRole("combobox"), lang);
      const code = await waitFor(() => canvas.getByRole("dialog").textContent ?? "");
      await expect(code).toContain("ROLTER_API_KEY");
      await expect(code).not.toContain("sk-rolter-");
    }
  },
};

// the entry point shipped as a bare `</>` glyph, so the dialog behind it was
// undiscoverable without clicking an anonymous icon (#963)
export const TriggerIsLabelled: Story = {
  play: async () => {
    const trigger = await screen().findByRole("button", { name: /copy as code/i });
    // the visible label, not just the accessible name the aria-label supplied
    await expect(trigger).toHaveTextContent(/copy as code/i);
    await expect(trigger).toHaveAttribute("title", expect.stringMatching(/copy as code/i));
  },
};
