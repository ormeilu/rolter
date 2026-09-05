import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor, within } from "storybook/test";
import { useTranslation } from "react-i18next";

import Keys from "./Keys";
import Providers from "./Providers";
import Security from "./Security";
import {
  Harness,
  expectForbidden,
  json,
  routes,
  scoped,
  type StoryRole,
} from "./story-harness";
import { NavSidebar, type NavItem } from "@/components/ui/nav-sidebar";
import type { SecuritySettingsDto } from "@/lib/api";
import { useCan } from "@/lib/can";
import { visibleNav, type NavDef } from "@/lib/nav";

// What each role sees of the same three screens (#1183).
//
// The dashboard used to render every control for every account and let the 403
// explain afterwards. These stories are the four answers the control plane
// gives — a viewer, a member, an admin and a superadmin — against the screens
// where the difference is most expensive: the keys anyone can try to mint, the
// providers anyone can try to add, and the deployment-wide security policy that
// is the admin token's alone.

const SECURITY: SecuritySettingsDto = {
  virtual_key_required: true,
  allowed_origins: ["https://app.example.com"],
  allowed_headers: ["x-request-id"],
  required_headers: {},
  auth_bypass_routes: ["/healthz"],
  dashboard_auth_enabled: true,
  dashboard_credential_ref: "ROLTER_DASHBOARD_SECRET",
  dashboard_secret_configured: true,
  updated_at: "2026-08-01T09:00:00Z",
};

// the screens under test only need to get *somewhere* renderable: what these
// stories assert is the control, not the list behind it
const empty = routes([]);
const securityStub = scoped(async () => json(SECURITY));

const ADD_KEY = /add virtual key/i;
const ADD_PROVIDER = /add provider/i;

/** Assert the create control is refused, and says what it would take. */
async function expectGatedOut(canvasElement: HTMLElement, name: RegExp) {
  const canvas = within(canvasElement);
  const button = await canvas.findByRole("button", { name });
  await waitFor(() => expect(button).toBeDisabled());
  // "disabled" alone is the same non-answer the 403 was: the control has to
  // name the role that would make it work
  await expect(button).toHaveAttribute("title", "Requires the Admin role");
}

/** Assert the create control is offered. */
async function expectOffered(canvasElement: HTMLElement, name: RegExp) {
  const canvas = within(canvasElement);
  const button = await canvas.findByRole("button", { name });
  await waitFor(() => expect(button).toBeEnabled());
}

const meta = {
  title: "Screens/Capability gating",
  parameters: { layout: "fullscreen" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

export const KeysAsViewer: Story = {
  render: () => (
    <Harness fetchStub={empty} role="viewer">
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectGatedOut(canvasElement, ADD_KEY);
  },
};

// a member may mint a key for themself on the Account screen, but minting one
// for a project is still an admin's — the two are different capabilities, and
// the button on this screen is the second one
export const KeysAsMember: Story = {
  render: () => (
    <Harness fetchStub={empty} role="member">
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectGatedOut(canvasElement, ADD_KEY);
  },
};

export const KeysAsAdmin: Story = {
  render: () => (
    <Harness fetchStub={empty} role="admin">
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectOffered(canvasElement, ADD_KEY);
  },
};

export const KeysAsSuperadmin: Story = {
  render: () => (
    <Harness fetchStub={empty} role="superadmin">
      <Keys />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectOffered(canvasElement, ADD_KEY);
  },
};

export const ProvidersAsViewer: Story = {
  render: () => (
    <Harness fetchStub={empty} role="viewer">
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectGatedOut(canvasElement, ADD_PROVIDER);
  },
};

export const ProvidersAsMember: Story = {
  render: () => (
    <Harness fetchStub={empty} role="member">
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectGatedOut(canvasElement, ADD_PROVIDER);
  },
};

export const ProvidersAsAdmin: Story = {
  render: () => (
    <Harness fetchStub={empty} role="admin">
      <Providers />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectOffered(canvasElement, ADD_PROVIDER);
  },
};

// the deployment-wide policy screens are the superadmin's alone: an admin is
// refused up front, before the request that would have been refused anyway
export const SecurityAsAdmin: Story = {
  render: () => (
    <Harness fetchStub={securityStub} role="admin">
      <Security />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectForbidden(canvasElement);
    // and it is not a failure a retry can change
    await expect(
      within(canvasElement).queryByRole("button", { name: /try again/i }),
    ).toBeNull();
  },
};

export const SecurityAsViewer: Story = {
  render: () => (
    <Harness fetchStub={securityStub} role="viewer">
      <Security />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await expectForbidden(canvasElement);
  },
};

export const SecurityAsSuperadmin: Story = {
  render: () => (
    <Harness fetchStub={securityStub} role="superadmin">
      <Security />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() =>
      expect(canvas.getByText("Password protect the dashboard")).toBeVisible(),
    );
  },
};

// the rail, built exactly as the shell builds it: `visibleNav` over the same
// `useCan` the screens ask
function GatedNav() {
  const { t } = useTranslation();
  const can = useCan();
  const toItem = (def: NavDef): NavItem => ({
    key: def.key,
    label: t(`nav.${def.key}`),
    icon: def.icon,
    children: def.children?.map(toItem),
  });
  return (
    <div className="flex h-[600px]">
      <NavSidebar
        brand="rolter"
        groups={[{ items: visibleNav(can).map(toItem) }]}
        activeKey="dashboard"
        searchable
      />
    </div>
  );
}

export const NavAsViewer: Story = {
  render: () => (
    <Harness fetchStub={empty} role="viewer">
      <GatedNav />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // a group whose every leaf is deployment-scoped is gone, not disabled: a
    // rail full of entries that all open the same refusal is a worse map of
    // the product than a shorter rail
    await waitFor(() => expect(canvas.queryByText("Alerting")).toBeNull());
    await expect(canvas.queryByText("Guardrails")).toBeNull();
    await expect(canvas.queryByText("Cluster Config")).toBeNull();
    // what a viewer may read is still there
    await expect(canvas.getByText("Governance")).toBeVisible();
    await expect(canvas.getByText("Playground")).toBeVisible();
  },
};

export const NavAsSuperadmin: Story = {
  render: () => (
    <Harness fetchStub={empty} role="superadmin">
      <GatedNav />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("Alerting")).toBeVisible());
    await expect(canvas.getByText("Guardrails")).toBeVisible();
    await expect(canvas.getByText("Cluster Config")).toBeVisible();
  },
};

// no provider above and no answer yet are the same case, and both render the
// dashboard as it always was: enabled, with the 403 as the backstop
export const NavUngated: Story = {
  render: () => (
    <Harness fetchStub={empty}>
      <GatedNav />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("Alerting")).toBeVisible();
  },
};

const ROLES: StoryRole[] = ["viewer", "member", "admin", "superadmin"];

// one frame with all four answers side by side, for the visual review the play
// functions cannot do
export const EveryRole: Story = {
  render: () => (
    <div className="flex flex-col gap-6">
      {ROLES.map((role) => (
        <Harness key={role} fetchStub={empty} role={role}>
          <Providers />
        </Harness>
      ))}
    </div>
  ),
};
