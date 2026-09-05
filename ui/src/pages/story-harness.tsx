import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import * as React from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";

import { AuthProvider } from "@/lib/auth";

// Shared fetch-stub harness for screen stories (#879).
//
// `Plugins.stories.tsx` established the shape — swap `globalThis.fetch`, clear
// the persisted scope, render the screen under a fresh QueryClient. Five more
// screens needed the same thing, and five hand-copied harnesses would be five
// places for the scope fixture to drift. This is that harness, factored out.
//
// Not a `.stories.tsx` file itself, so Storybook does not try to render it as a
// screen and `check:literals` does not scan it for copy.

export type FetchStub = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export const ORG = { id: "org-1", name: "Rolter", slug: "rolter", created_at: "2026-01-01T00:00:00Z" };
export const TEAM = { id: "team-1", org_id: "org-1", name: "Platform", created_at: "2026-01-01T00:00:00Z" };
export const PROJECT = { id: "project-1", team_id: "team-1", name: "Gateway", created_at: "2026-01-01T00:00:00Z" };

export const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

/**
 * The org/team/project chain every scoped screen resolves before it can load.
 *
 * Matched on the whole path rather than on a fragment: a screen's own endpoint
 * often *contains* one of these segments — `/api/v1/projects/{id}/virtual-keys`
 * is the obvious one — and a substring match would answer it with the project
 * list, leaving the screen rendering scope rows and the story asserting nothing
 * it meant to.
 */
export const scopeResponse = (url: string): Response | null => {
  const path = new URL(url, "http://localhost").pathname;
  if (path === "/api/v1/orgs") return json([ORG]);
  if (/^\/api\/v1\/orgs\/[^/]+\/teams$/.test(path)) return json([TEAM]);
  if (/^\/api\/v1\/teams\/[^/]+\/projects$/.test(path)) return json([PROJECT]);
  return null;
};

/** A stub that resolves the scope and then answers `handler`. */
export function scoped(handler: FetchStub): FetchStub {
  return async (input, init) => scopeResponse(String(input)) ?? handler(input, init);
}

/** A stub that never settles, for the loading state. */
export const pending: FetchStub = scoped(() => new Promise<Response>(() => {}));

/**
 * Route by URL fragment. Entries are matched in order, so a longer path can be
 * listed before the prefix it shares.
 */
export function routes(table: [string, () => unknown][], status = 200): FetchStub {
  return scoped(async (input) => {
    const url = String(input);
    for (const [fragment, body] of table) {
      if (url.includes(fragment)) return json(body(), status);
    }
    return json([], status);
  });
}

export function Harness({
  fetchStub,
  children,
}: {
  fetchStub: FetchStub;
  children: React.ReactNode;
}) {
  const original = React.useRef<typeof globalThis.fetch | null>(null);
  const client = React.useMemo(() => {
    original.current ??= globalThis.fetch;
    localStorage.removeItem("rolter.scope");
    globalThis.fetch = fetchStub as typeof globalThis.fetch;
    // no retries: a story asserting an error state should not wait out a
    // backoff schedule before the screen admits the request failed
    return new QueryClient({ defaultOptions: { queries: { retry: false } } });
  }, [fetchStub]);
  React.useEffect(
    () => () => {
      if (original.current) globalThis.fetch = original.current;
    },
    [],
  );
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

/**
 * An `AuthProvider` that boots with a session token already in localStorage,
 * the way a reloaded tab does (#1196).
 *
 * The token is written during render, before the provider mounts and reads it,
 * and the whole session is cleared again on unmount so a story that leaves a
 * dead token behind cannot change what the next story sees.
 */
export function StaleSession({
  token = "stale-session-token",
  email = "anya@acme.co",
  children,
}: {
  token?: string;
  email?: string;
  children: React.ReactNode;
}) {
  React.useState(() => {
    localStorage.setItem("rolter.session.token", token);
    localStorage.setItem("rolter.session.email", email);
    return null;
  });
  React.useEffect(
    () => () => {
      localStorage.removeItem("rolter.session.token");
      localStorage.removeItem("rolter.session.email");
      localStorage.removeItem("rolter.session.user");
    },
    [],
  );
  return <AuthProvider>{children}</AuthProvider>;
}

/**
 * Run `body` with `window.confirm` answering `answer`, then restore it.
 *
 * The editor sheets guard discarding a dirty draft with `window.confirm`, which
 * is a real modal in a browser and would hang the test runner. Stubbing it is
 * also the only way to assert *both* answers — that "cancel" keeps the sheet
 * open is the half a manual click-through never checks.
 */
export async function withConfirm(answer: boolean, body: () => Promise<void>): Promise<void> {
  const original = window.confirm;
  window.confirm = () => answer;
  try {
    await body();
  } finally {
    window.confirm = original;
  }
}

/**
 * Click a button once it is actually clickable.
 *
 * `findByRole` waits for the element to *exist*, not to be enabled, and most of
 * these screens disable their primary action until the org/team/project chain
 * has resolved — three sequential requests. Clicking in between throws
 * `pointer-events: none`, which reads like a layout bug and is really a race.
 */
export async function clickWhenEnabled(
  container: HTMLElement,
  name: RegExp | string,
): Promise<void> {
  const canvas = within(container);
  const button = await canvas.findByRole("button", { name });
  await waitFor(() => expect(button).toBeEnabled());
  await userEvent.click(button);
}

/** The open editor sheet. Sheets portal to the body, not into the canvas. */
export function sheet(): HTMLElement {
  return within(document.body).getByRole("dialog");
}

/** Assert the sheet closed. */
export async function expectSheetClosed(): Promise<void> {
  await waitFor(() => expect(within(document.body).queryByRole("dialog")).not.toBeInTheDocument());
}

/**
 * Close the sheet and assert it did *not* ask to discard — the untouched-draft
 * case. A confirm on a form nobody edited trains people to click through the
 * one that matters.
 */
export async function expectClosesWithoutPrompting(closeLabel = "Cancel"): Promise<void> {
  let asked = false;
  const original = window.confirm;
  window.confirm = () => {
    asked = true;
    return true;
  };
  try {
    await userEvent.click(within(sheet()).getByRole("button", { name: closeLabel }));
    await expectSheetClosed();
    expect(asked).toBe(false);
  } finally {
    window.confirm = original;
  }
}
