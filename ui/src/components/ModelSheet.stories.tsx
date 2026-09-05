import type { Meta, StoryObj } from "@storybook/react-vite";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { ModelSheet, type ModelSheetMode } from "./ModelSheet";
import {
  Harness,
  ORG,
  PROJECT,
  expectClosesWithoutPrompting,
  expectSheetClosed,
  json,
  recording,
  sheet,
  withConfirm,
  type FetchStub,
} from "@/pages/story-harness";
import type { EffectiveModelDto, ProviderRow, RouteRow } from "@/lib/api";

const PROVIDERS: ProviderRow[] = [
  {
    id: "prov-1",
    org_id: ORG.id,
    name: "openai-prod",
    slug: "openai-prod",
    kind: "openai",
    api_base: "https://api.openai.com",
    egress_proxies: [],
    created_at: "2026-01-01T00:00:00Z",
  },
  {
    id: "prov-2",
    org_id: ORG.id,
    name: "vllm-cluster",
    slug: "vllm-cluster",
    kind: "openai_compatible",
    api_base: "http://vllm.internal:8000",
    egress_proxies: [],
    created_at: "2026-01-01T00:00:00Z",
  },
];

const ROUTE: RouteRow = {
  id: "route-1",
  project_id: PROJECT.id,
  model: "gpt-4o",
  strategy: "round_robin",
  enabled: true,
  params: { temperature: 0.7 },
  param_policy: { mode: "allow", allow: [], deny: [] },
  advanced: {},
  created_at: "2026-02-01T00:00:00Z",
};

const MODELS: EffectiveModelDto[] = [
  { model: "gpt-4o", strategy: "round_robin", targets: 1, source: "db" },
  { model: "fake-llm", strategy: "round_robin", targets: 1, source: "config" },
];

const TARGETS = [
  {
    id: "tgt-1",
    route_id: ROUTE.id,
    provider_id: "prov-1",
    upstream_model: null,
    weight: 1,
    position: 0,
  },
];

/** everything the sheet reads on open; the rbac chip sources are best-effort */
const backing: FetchStub = async (input) => {
  const url = String(input);
  if (url.includes("/routes/route-1/targets")) return json(TARGETS);
  if (url.includes("/model-prices")) return json([]);
  if (url.includes("/currency")) return json({ settlement: "USD", codes: ["USD", "EUR"] });
  if (url.includes("/teams")) return json([]);
  if (url.includes("/virtual-keys")) return json([]);
  if (url.includes("/users")) return json([]);
  return json({ id: "route-new", model: "new" });
};

/** the recorder the story under way installed, read back by its play function */
let calls: ReturnType<typeof recording>;

function Stage({
  mode,
  route,
  configModel,
  stub = backing,
}: {
  mode: ModelSheetMode;
  route?: RouteRow | null;
  configModel?: EffectiveModelDto | null;
  stub?: FetchStub;
}) {
  const [open, setOpen] = React.useState(true);
  // a ref, not `useMemo`: a story that passes an inline stub changes its
  // identity on every render, and a memo keyed on it would hand each render a
  // fresh recorder — with the requests the last one saw thrown away
  const recorder = React.useRef<ReturnType<typeof recording> | null>(null);
  if (!recorder.current) {
    recorder.current = recording(stub);
    calls = recorder.current;
  }
  return (
    <Harness fetchStub={recorder.current.stub}>
      <ModelSheet
        open={open}
        mode={mode}
        onOpenChange={setOpen}
        projectId={PROJECT.id}
        orgId={ORG.id}
        providers={PROVIDERS}
        route={route}
        configModel={configModel}
        models={MODELS}
        routes={[ROUTE]}
        onDone={() => {}}
      />
    </Harness>
  );
}

// the sheet seeds its draft in an effect, so every play function waits for the
// seed before it types: an edit landing first would be overwritten by it
async function seeded(dialog: ReturnType<typeof within>): Promise<void> {
  await waitFor(() => expect(dialog.getByLabelText("Provider")).toHaveValue("prov-1"));
}

const meta = {
  title: "Overlays/ModelSheet",
  component: ModelSheet,
  parameters: { layout: "fullscreen" },
  // every story renders through `Stage`, which owns the props; these satisfy
  // the required-prop contract for the docs page
  args: {
    open: true,
    mode: "add" as const,
    onOpenChange: () => {},
    projectId: PROJECT.id,
    orgId: ORG.id,
    providers: PROVIDERS,
    models: MODELS,
    routes: [ROUTE],
    onDone: () => {},
  },
} satisfies Meta<typeof ModelSheet>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A blank draft: nothing is valid yet, and the sheet says which fields. */
export const Add: Story = {
  render: () => <Stage mode="add" />,
  play: async () => {
    const dialog = within(sheet());
    await expect(dialog.getByText("Add model")).toBeVisible();
    // the primary action is withheld rather than disabled while the draft is
    // incomplete; the two required fields carry the reason instead
    await expect(dialog.queryByRole("button", { name: "Add model" })).not.toBeInTheDocument();
    // `getAll`: the sheet states each error under its field *and* repeats the
    // set in a summary above the footer
    await expect(dialog.getAllByText(/Pick the upstream provider/).length).toBeGreaterThan(0);
    // "duplicate from" is offered only where there is something to duplicate
    await expect(dialog.getByLabelText("Duplicate from")).toBeVisible();
  },
};

/**
 * Edit waits for the route's target and the price table before it seeds the
 * draft — seeding early would show an empty provider on a model that has one.
 */
export const EditLoading: Story = {
  render: () => <Stage mode="edit" route={ROUTE} stub={() => new Promise<Response>(() => {})} />,
  play: async () => {
    const dialog = within(sheet());
    await waitFor(() => expect(dialog.getAllByRole("status").length).toBeGreaterThan(0));
  },
};

export const Edit: Story = {
  render: () => <Stage mode="edit" route={ROUTE} />,
  play: async () => {
    const dialog = within(sheet());
    await seeded(dialog);
    await expect(dialog.getByLabelText("Upstream model name")).toHaveValue("gpt-4o");
    // renaming is not supported yet, and the field says so rather than
    // accepting an edit the control plane would drop
    await expect(dialog.getByLabelText("Upstream model name")).toBeDisabled();
  },
};

/**
 * A config-owned model. It is always present and cannot be edited — the
 * control plane answers 409 — so the sheet is a reference view that says why
 * rather than a form that fails on save.
 */
export const ViewConfigModel: Story = {
  render: () => <Stage mode="view" configModel={MODELS[1]} />,
  play: async () => {
    const dialog = within(sheet());
    await expect(dialog.getByText("Model details")).toBeVisible();
    await expect(dialog.getByText(/Read-only config model/)).toBeVisible();
    await expect(dialog.queryByRole("button", { name: "Save model" })).not.toBeInTheDocument();
    await expect(dialog.getByLabelText("Provider")).toBeDisabled();
  },
};

/**
 * A public name already in the catalog. Two routes answering the same name is
 * ambiguous, so it is caught here rather than by whichever one the gateway
 * happens to resolve first.
 */
export const NameConflict: Story = {
  render: () => <Stage mode="add" />,
  play: async () => {
    const dialog = within(sheet());
    await seeded(dialog);
    await userEvent.type(dialog.getByLabelText("Upstream model name"), "gpt-4o");
    await expect(dialog.getAllByText(/already exists/).length).toBeGreaterThan(0);
    await expect(dialog.queryByRole("button", { name: "Add model" })).not.toBeInTheDocument();
  },
};

/** A base URL that is not a URL is refused before it reaches a provider. */
export const InvalidBaseUrl: Story = {
  render: () => <Stage mode="add" />,
  play: async () => {
    const dialog = within(sheet());
    await seeded(dialog);
    await userEvent.type(dialog.getByLabelText("Base URL override"), "vllm.internal:8000");
    await expect(dialog.getAllByText(/must start with http/).length).toBeGreaterThan(0);
  },
};

/**
 * Adding a model is a route plus a target, in that order: the target needs the
 * id the route creation returns.
 */
export const AddsAModel: Story = {
  render: () => <Stage mode="add" />,
  play: async () => {
    const dialog = within(sheet());
    await seeded(dialog);
    await userEvent.selectOptions(dialog.getByLabelText("Provider"), "prov-2");
    await userEvent.type(dialog.getByLabelText("Upstream model name"), "llama-3.1-70b");
    await userEvent.click(dialog.getByRole("button", { name: "Add model" }));
    const route = (await calls.expectSentBody("POST", `/projects/${PROJECT.id}/routes`)) as {
      model: string;
    };
    await expect(route.model).toBe("llama-3.1-70b");
    const target = (await calls.expectSentBody("POST", "/routes/route-new/targets")) as {
      provider_id: string;
    };
    await expect(target.provider_id).toBe("prov-2");
  },
};

/** An untouched draft closes without asking whether to discard it. */
export const ClosesCleanWithoutPrompting: Story = {
  render: () => <Stage mode="edit" route={ROUTE} />,
  play: async () => {
    const dialog = within(sheet());
    await seeded(dialog);
    await expectClosesWithoutPrompting();
  },
};

/** A dirty draft asks first, and "cancel" leaves the edit where it was. */
export const DiscardGuardKeepsTheDraft: Story = {
  render: () => <Stage mode="add" />,
  play: async () => {
    const dialog = within(sheet());
    await seeded(dialog);
    await userEvent.type(dialog.getByLabelText("Upstream model name"), "llama-3.1-70b");
    await withConfirm(false, async () => {
      await userEvent.click(dialog.getByRole("button", { name: /close/i }));
    });
    await expect(dialog.getByLabelText("Upstream model name")).toHaveValue("llama-3.1-70b");
  },
};

/** And "discard" closes it. */
export const DiscardGuardThrowsItAway: Story = {
  render: () => <Stage mode="add" />,
  play: async () => {
    const dialog = within(sheet());
    await seeded(dialog);
    await userEvent.type(dialog.getByLabelText("Upstream model name"), "llama-3.1-70b");
    await withConfirm(true, async () => {
      await userEvent.click(dialog.getByRole("button", { name: /close/i }));
    });
    await expectSheetClosed();
  },
};
