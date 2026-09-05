import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import Rbac from "./Rbac";
import { Harness, json, pending, routes, scoped } from "./story-harness";
import type { MembershipRow, RbacMatrix } from "@/lib/api";

// a slice of the real CAPABILITIES table, chosen for the four things a cell can
// say: a plain minimum role, a superadmin-only deployment setting, an action
// that does not exist for the resource at all, and one open to any
// authenticated caller. The absent actions are absent, not null — the control
// plane filters them out, which is exactly why they must not read as "denied".
const MATRIX: RbacMatrix = {
  roles: [
    { role: "viewer", rank: 0 },
    { role: "member", rank: 1 },
    { role: "admin", rank: 2 },
  ],
  resources: [
    {
      resource: "runtime_settings",
      scope: "deployment",
      actions: [
        { action: "read", minimum_role: null, superadmin_only: true, authenticated_only: false },
        { action: "update", minimum_role: null, superadmin_only: true, authenticated_only: false },
      ],
    },
    {
      resource: "org",
      scope: "org",
      actions: [
        { action: "read", minimum_role: "viewer", superadmin_only: false, authenticated_only: false },
        { action: "create", minimum_role: null, superadmin_only: true, authenticated_only: false },
        { action: "delete", minimum_role: "admin", superadmin_only: false, authenticated_only: false },
      ],
    },
    {
      resource: "provider",
      scope: "org",
      actions: [
        { action: "read", minimum_role: "viewer", superadmin_only: false, authenticated_only: false },
        { action: "create", minimum_role: "admin", superadmin_only: false, authenticated_only: false },
        { action: "update", minimum_role: "admin", superadmin_only: false, authenticated_only: false },
        { action: "delete", minimum_role: "admin", superadmin_only: false, authenticated_only: false },
      ],
    },
    {
      // append-only: it has a read and nothing else, for anyone
      resource: "audit_log",
      scope: "org",
      actions: [
        { action: "read", minimum_role: "admin", superadmin_only: false, authenticated_only: false },
      ],
    },
    {
      // a global catalog carrying no tenant's data (#766)
      resource: "provider_kind",
      scope: "org",
      actions: [
        { action: "read", minimum_role: null, superadmin_only: false, authenticated_only: true },
      ],
    },
    {
      resource: "project",
      scope: "team",
      actions: [
        { action: "read", minimum_role: "viewer", superadmin_only: false, authenticated_only: false },
        { action: "create", minimum_role: "admin", superadmin_only: false, authenticated_only: false },
        { action: "delete", minimum_role: "admin", superadmin_only: false, authenticated_only: false },
      ],
    },
    {
      resource: "virtual_key",
      scope: "project",
      actions: [
        { action: "read", minimum_role: "viewer", superadmin_only: false, authenticated_only: false },
        { action: "create", minimum_role: "member", superadmin_only: false, authenticated_only: false },
        { action: "update", minimum_role: "member", superadmin_only: false, authenticated_only: false },
        { action: "delete", minimum_role: "member", superadmin_only: false, authenticated_only: false },
      ],
    },
  ],
  custom_roles: [],
};

// a viewer that may also mint keys, plus a grant naming a resource this build
// retired — reported rather than hidden, because it silently allows nothing
const WITH_CUSTOM_ROLE: RbacMatrix = {
  ...MATRIX,
  custom_roles: [
    {
      id: "role-1",
      slug: "support-engineer",
      name: "Support engineer",
      description: "Read everything, mint keys for a customer",
      base_role: "viewer",
      base_rank: 0,
      grants: [{ resource: "virtual_key", action: "create" }],
      unknown_grants: [{ resource: "legacy_widget", action: "read" }],
    },
  ],
};

const MEMBERSHIPS: MembershipRow[] = [
  { id: "m-1", user_id: "u-1", org_id: "org-1", team_id: null, project_id: null, role: "admin", created_at: "2026-01-01T00:00:00Z" },
  { id: "m-2", user_id: "u-2", org_id: "org-1", team_id: null, project_id: null, role: "admin", created_at: "2026-01-01T00:00:00Z" },
  { id: "m-3", user_id: "u-3", org_id: "org-1", team_id: null, project_id: null, role: "member", created_at: "2026-01-01T00:00:00Z" },
];

const stub = (matrix: RbacMatrix) =>
  routes([
    ["/rbac/matrix", () => matrix],
    ["/memberships", () => MEMBERSHIPS],
  ]);

const meta = {
  title: "Screens/Rbac",
  component: Rbac,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Rbac>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  render: () => (
    <Harness fetchStub={stub(MATRIX)}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    // resources come from the server's table, grouped by the scope they live in
    await expect(await canvas.findByText("runtime_settings")).toBeVisible();
    await expect(canvas.getByText("virtual_key")).toBeVisible();
    await expect(canvas.getByText("Deployment-wide")).toBeVisible();
    await expect(canvas.getByText("Project")).toBeVisible();
    await expect(canvas.getByText("7 resources")).toBeVisible();

    // one column per built-in role, with the live membership count under it
    await expect(canvas.getByText("admin")).toBeVisible();
    await expect(canvas.getByText("2 members")).toBeVisible();
    await expect(canvas.getByText("1 member")).toBeVisible();
    await expect(canvas.getByText("0 members")).toBeVisible();
  },
};

// the distinction #1178 exists for: an action a resource does not have is "not
// applicable" for everyone, and a deployment setting is nobody's org role
export const MarksNotApplicableAndSuperadmin: Story = {
  render: () => (
    <Harness fetchStub={stub(MATRIX)}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findByText("audit_log");

    // an append-only log has no create/update/delete for any of the three roles
    await expect(canvas.getAllByTitle("Create — not applicable")).toHaveLength(9);
    // runtime settings are superadmin-only, so no org role reaches them
    await expect(canvas.getAllByTitle("Update — superadmin only")).toHaveLength(3);
    // and a member may not delete a provider, which *is* a denial
    await expect(canvas.getAllByTitle("Delete — denied")).not.toHaveLength(0);
    // a catalog open to any authenticated caller is allowed all the way down
    await expect(canvas.getAllByTitle("Read — allowed").length).toBeGreaterThan(3);
  },
};

export const WithCustomRole: Story = {
  render: () => (
    <Harness fetchStub={stub(WITH_CUSTOM_ROLE)}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    // the org-defined role sits beside the built-ins, ranked by its base role
    await expect(await canvas.findByText("Support engineer")).toBeVisible();
    await expect(canvas.getByText("granted by access profile")).toBeVisible();
    await expect(canvas.getByText("4 roles")).toBeVisible();

    // it is a viewer plus exactly one pair, shown as its own state
    await expect(
      canvas.getAllByTitle("Create — granted by this custom role"),
    ).toHaveLength(1);

    // a grant naming a resource this build retired is surfaced, not hidden
    await expect(canvas.getByText(/does not define/)).toBeVisible();
  },
};

export const Loading: Story = {
  render: () => (
    <Harness fetchStub={pending}>
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    // the placeholder is the shape of the table that is coming, not a spinner
    await expect(canvasElement.querySelectorAll(".rl-skeleton").length).toBeGreaterThan(10);
  },
};

// any authenticated principal may read the matrix, so the failure worth showing
// is the org half: custom roles need a membership in the org being asked about
export const Forbidden: Story = {
  render: () => (
    <Harness
      fetchStub={scoped(async (input) =>
        String(input).includes("/rbac/matrix")
          ? json({ error: { message: "forbidden" } }, 403)
          : json([]),
      )}
    >
      <Rbac />
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      await canvas.findByText(/do not have access to the permission matrix/i),
    ).toBeVisible();
  },
};
