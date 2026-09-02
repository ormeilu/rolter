import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";

import Account from "./Account";
import {
  Harness,
  clickWhenEnabled,
  expectClosesWithoutPrompting,
  json,
  pending,
  scoped,
  sheet,
  withConfirm,
  type FetchStub,
} from "./story-harness";
import type { MintedKey, MyUsageRow, OwnedKeyRow } from "@/lib/api";

const KEYS: OwnedKeyRow[] = [
  {
    id: "vk-1",
    project_id: "project-1",
    project_name: "Gateway",
    org_name: "Rolter",
    key_prefix: "sk-rolter-laptop",
    name: "my laptop",
    models: ["gpt-4o"],
    disabled: false,
    expires_at: null,
    created_at: "2026-07-01T00:00:00Z",
  },
  {
    id: "vk-2",
    project_id: "project-1",
    project_name: "Gateway",
    org_name: "Rolter",
    key_prefix: "sk-rolter-retired",
    name: null,
    models: [],
    disabled: true,
    expires_at: null,
    created_at: "2026-06-01T00:00:00Z",
  },
];

const USAGE: MyUsageRow[] = [
  { virtual_key_id: "vk-1", requests: 1204, tokens: 903_112, cost_usd: "12.34", errors: 3 },
];

const MINTED: MintedKey = {
  id: "vk-3",
  project_id: "project-1",
  key_hash: "hash-3",
  key_prefix: "sk-rolter-ci-runner",
  name: "ci runner",
  models: [],
  disabled: false,
  created_at: "2026-08-01T00:00:00Z",
  key: "sk-rolter-plaintext-shown-once",
};

/**
 * The screen runs two independent queries — keys and usage — and the usage one
 * is allowed to fail on its own, so every stub has to answer both.
 */
const account = (
  keys: () => Response,
  usage: () => Response = () => json({ data: USAGE }),
): FetchStub =>
  scoped(async (input) => (String(input).includes("/me/usage") ? usage() : keys()));

const loaded = account(() => json(KEYS));

const meta = {
  title: "Screens/Account",
  component: Account,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Account>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText("my laptop")).toBeInTheDocument();
    // a key with no usage row still renders, with the window spelled out —
    // "nothing recorded" and "analytics is down" must not look the same
    await expect(canvas.getByText(/no usage in the last 7 days/i)).toBeInTheDocument();
    await expect(canvas.getByText(/req ·/)).toBeInTheDocument();
  },
};

export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Account />
    </Harness>
  ),
};

export const Empty: Story = {
  render: () => (
    <Harness fetchStub={account(() => json([]))}>
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText(/haven't minted a virtual key yet/i)).toBeInTheDocument();
  },
};

export const Forbidden: Story = {
  render: () => (
    <Harness fetchStub={account(() => json({ error: { message: "forbidden" } }, 403))}>
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText(/do not have access to your keys/i)).toBeInTheDocument();
  },
};

/**
 * ClickHouse is optional, so `/me/usage` answering 503 is a supported
 * deployment rather than a fault. The keys must still render: losing the whole
 * self-service panel because the analytics store is absent would strand every
 * user who needs to rotate a key.
 */
export const AnalyticsUnavailable: Story = {
  render: () => (
    <Harness
      fetchStub={account(
        () => json(KEYS),
        () => json({ error: { message: "analytics not configured" } }, 503),
      )}
    >
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText("my laptop")).toBeInTheDocument();
    await expect(canvas.getAllByText(/analytics not configured/i)).toHaveLength(KEYS.length);
  },
};

export const MintsAKey: Story = {
  render: () => (
    <Harness
      fetchStub={scoped(async (input, init) => {
        const url = String(input);
        if (init?.method === "POST") return json(MINTED, 201);
        if (url.includes("/me/usage")) return json({ data: USAGE });
        return json(KEYS);
      })}
    >
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /generate virtual key/i);
    const form = sheet();
    await userEvent.type(within(form).getByLabelText("Name"), "ci runner");
    await userEvent.click(within(form).getByRole("button", { name: "Mint" }));
    // the plaintext is shown exactly once, right here; losing this dialog means
    // the user never gets the secret they just created
    await waitFor(() =>
      expect(within(document.body).getByText(MINTED.key)).toBeInTheDocument(),
    );
    await userEvent.click(within(document.body).getByRole("button", { name: "Done" }));
    await waitFor(() =>
      expect(within(document.body).queryByText(MINTED.key)).not.toBeInTheDocument(),
    );
  },
};

/**
 * Rotation reaches the same reveal dialog by a different path — the card's own
 * mutation rather than the mint sheet — and that second entry point is the one
 * a refactor of the sheet would quietly drop.
 */
export const RotatingAKeyRevealsTheNewSecret: Story = {
  render: () => (
    <Harness
      fetchStub={scoped(async (input, init) => {
        const url = String(input);
        if (init?.method === "POST") return json(MINTED);
        if (url.includes("/me/usage")) return json({ data: USAGE });
        return json(KEYS);
      })}
    >
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const [rotate] = await canvas.findAllByRole("button", { name: /rotate/i });
    await userEvent.click(rotate);
    await waitFor(() =>
      expect(within(document.body).getByText(MINTED.key)).toBeInTheDocument(),
    );
  },
};

/** The dirty guard from #868: an untouched draft closes without a confirm. */
export const AnUntouchedMintFormClosesWithoutPrompting: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /generate virtual key/i);
    await expectClosesWithoutPrompting();
  },
};

export const AnEditedMintFormPromptsBeforeDiscarding: Story = {
  render: () => (
    <Harness fetchStub={loaded}>
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /generate virtual key/i);
    const form = sheet();
    await userEvent.type(within(form).getByLabelText("Name"), "half typed");

    await withConfirm(false, async () => {
      await userEvent.click(within(form).getByRole("button", { name: "Cancel" }));
      // declining keeps the sheet, and the typing, alive
      await expect(within(document.body).getByRole("dialog")).toBeInTheDocument();
      await expect(within(form).getByLabelText("Name")).toHaveValue("half typed");
    });
  },
};

/**
 * The mint sheet's whole point after #945 is that the two insecure choices are
 * no longer the ones you get by doing nothing: the key must be named, and it
 * expires in 30 days unless you say otherwise.
 */
export const MintRequiresANameAndDefaultsToAFiniteLife: Story = {
  render: () => (
    <Harness fetchStub={scoped(async () => json(KEYS))}>
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /generate virtual key/i);
    const form = sheet();
    // nothing typed: minting is refused before a round trip is spent
    await expect(within(form).getByRole("button", { name: "Mint" })).toBeDisabled();
    // whitespace is not a name either
    await userEvent.type(within(form).getByLabelText("Name"), "   ");
    await expect(within(form).getByRole("button", { name: "Mint" })).toBeDisabled();
    await userEvent.type(within(form).getByLabelText("Name"), "ci runner");
    await expect(within(form).getByRole("button", { name: "Mint" })).toBeEnabled();

    // the default expiry is finite, and the reach panel says so in a date
    await expect(within(form).getByLabelText("Expires")).toHaveValue("30");
    await expect(
      within(form).getByText(/every model this project can route to/i),
    ).toBeInTheDocument();
    await expect(within(form).getByText(/^Until /)).toBeInTheDocument();
  },
};

/**
 * "Never expires" stays available — it is a legitimate choice for a key an
 * operator has other controls over — but it has to be picked, and picking it
 * says what it costs.
 */
export const NeverExpiringIsADeliberateChoice: Story = {
  render: () => (
    <Harness
      fetchStub={scoped(async (_input, init) => {
        if (init?.method === "POST") {
          // the body is what the assertion is really about: no TTL at all,
          // rather than a zero or an empty string the server would reject
          const body = JSON.parse(String(init.body));
          if (body.expires_in_days !== undefined) return json({ error: { message: "sent a ttl" } }, 400);
          if (!body.name) return json({ error: { message: "sent no name" } }, 400);
          return json(MINTED, 201);
        }
        return json(KEYS);
      })}
    >
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /generate virtual key/i);
    const form = sheet();
    await userEvent.type(within(form).getByLabelText("Name"), "build box");
    await userEvent.selectOptions(within(form).getByLabelText("Expires"), "never");
    await expect(within(form).getByText(/until someone revokes it/i)).toBeInTheDocument();
    await expect(within(form).getByText(/forever, until revoked/i)).toBeInTheDocument();
    await userEvent.click(within(form).getByRole("button", { name: "Mint" }));
    await waitFor(() =>
      expect(within(document.body).getByText(MINTED.key)).toBeInTheDocument(),
    );
  },
};

/**
 * A server-side rejection has to land on the sheet rather than vanishing — the
 * name rule is enforced in two places and the operator must see which one spoke.
 */
export const MintRejectionIsShownOnTheSheet: Story = {
  render: () => (
    <Harness
      fetchStub={scoped(async (_input, init) => {
        if (init?.method === "POST")
          return json({ error: { message: "virtual key name is required" } }, 400);
        return json(KEYS);
      })}
    >
      <Account />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await clickWhenEnabled(canvasElement, /generate virtual key/i);
    const form = sheet();
    await userEvent.type(within(form).getByLabelText("Name"), "rejected");
    await userEvent.click(within(form).getByRole("button", { name: "Mint" }));
    await waitFor(() =>
      expect(within(form).getByText(/virtual key name is required/i)).toBeInTheDocument(),
    );
    // and the sheet stays open, so the operator can fix it in place
    await expect(within(form).getByLabelText("Name")).toBeInTheDocument();
  },
};
