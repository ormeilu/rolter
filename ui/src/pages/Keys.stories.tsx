import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";

import Keys from "./Keys";
import {
  Harness,
  clickWhenEnabled,
  expectClosesWithoutPrompting,
  expectSheetClosed,
  json,
  pending,
  scoped,
  sheet,
  withConfirm,
  type FetchStub,
} from "./story-harness";
import type { VirtualKeyRow } from "@/lib/api";

const KEYS: VirtualKeyRow[] = [
  {
    id: "vk-1",
    project_id: "project-1",
    key_hash: "hash-1",
    key_prefix: "sk-rolter-backend",
    name: "backend service",
    models: ["gpt-4o", "claude-sonnet"],
    disabled: false,
    expires_at: null,
    cache_enabled: null,
    created_at: "2026-07-01T00:00:00Z",
  },
  {
    id: "vk-2",
    project_id: "project-1",
    key_hash: "hash-2",
    key_prefix: "sk-rolter-laptop",
    name: "revoked laptop",
    models: [],
    disabled: true,
    expires_at: "2026-12-31T00:00:00Z",
    cache_enabled: false,
    created_at: "2026-06-01T00:00:00Z",
  },
];

const withKeys = (keys: VirtualKeyRow[], status = 200): FetchStub =>
  scoped(async () => json(status === 200 ? keys : { error: { message: "forbidden" } }, status));

const meta = {
  title: "Screens/Keys",
  component: Keys,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Keys>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={withKeys(KEYS)}>
      <Keys />
    </Harness>
  ),
};

export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Keys />
    </Harness>
  ),
};

export const Empty: Story = {
  render: () => (
    <Harness fetchStub={withKeys([])}>
      <Keys />
    </Harness>
  ),
};

export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={withKeys([], 403)}>
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // a 403 is a permission problem, not a data problem (#962): the screen
    // must say so rather than blame the list it could not read
    await expect(
      await canvas.findByText(/do not have access to virtual keys/i),
    ).toBeInTheDocument();
    // and must not offer a retry that cannot possibly succeed
    await expect(canvas.queryByRole("button", { name: /try again/i })).toBeNull();
  },
};

export const CreatesAKey: Story = {
  render: () => (
    <Harness
      fetchStub={scoped(async (_input, init) =>
        init?.method === "POST"
          ? json({ ...KEYS[0], key: "sk-rolter-plaintext-shown-once" }, 201)
          : json(KEYS),
      )}
    >
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /add virtual key/i);
    const form = sheet();
    await userEvent.type(within(form).getByLabelText("Name (optional)"), "ci runner");
    await userEvent.click(within(form).getByRole("button", { name: "Create" }));
    // the plaintext key is shown exactly once, right after creation — losing
    // that dialog means the caller never gets their secret
    await waitFor(() =>
      expect(within(document.body).getByText("sk-rolter-plaintext-shown-once")).toBeInTheDocument(),
    );
  },
};

/**
 * The dirty guard #868 introduced. A blank draft is *not* dirty, so closing it
 * must not prompt — a confirm on an untouched form trains people to click
 * through the one that matters.
 */
export const ClosesACleanDraftWithoutPrompting: Story = {
  render: () => (
    <Harness fetchStub={withKeys(KEYS)}>
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /add virtual key/i);
    await expectClosesWithoutPrompting();
  },
};

export const KeepsADirtyDraftWhenDiscardIsDeclined: Story = {
  render: () => (
    <Harness fetchStub={withKeys(KEYS)}>
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /add virtual key/i);
    const form = sheet();
    await userEvent.type(within(form).getByLabelText("Name (optional)"), "half typed");

    await withConfirm(false, async () => {
      await userEvent.click(within(form).getByRole("button", { name: "Cancel" }));
      // declining the discard keeps the sheet — and the typing — alive
      await expect(within(document.body).getByRole("dialog")).toBeInTheDocument();
      await expect(within(form).getByLabelText("Name (optional)")).toHaveValue("half typed");
    });

    await withConfirm(true, async () => {
      await userEvent.click(within(form).getByRole("button", { name: "Cancel" }));
      await expectSheetClosed();
    });
  },
};
