import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";

import Login from "./Login";
import { Harness, json, type FetchStub } from "./story-harness";
import { AuthProvider } from "@/lib/auth";

/**
 * The login screen used to answer every failure the same way: it signed the
 * user in through the email-only gate, with no session token. A wrong
 * password, a locked account and an SSO-only org were indistinguishable from
 * success until the first `/me/*` call failed for a reason nothing on screen
 * explained. These stories are what stops that coming back (#1160).
 */
const meta: Meta<typeof Login> = {
  title: "Screens/Login",
  component: Login,
};
export default meta;
type Story = StoryObj<typeof Login>;

const METHODS = { password: true, sso: [] };

/** Password login answers `methods`, then `login` fails with `code`. */
function loginFails(
  status: number,
  code: string,
  extra: { retryAfter?: string } = {},
): FetchStub {
  return async (input) => {
    const url = String(input);
    if (url.includes("/auth/methods")) return json(METHODS);
    if (url.includes("/auth/login")) {
      return new Response(
        JSON.stringify({ error: { message: "refused", code } }),
        {
          status,
          headers: {
            "Content-Type": "application/json",
            ...(extra.retryAfter ? { "Retry-After": extra.retryAfter } : {}),
          },
        },
      );
    }
    return json({});
  };
}

// the form starts empty (it once shipped a fake demo account pre-filled), so a
// sign-in has to type an account first — `required` blocks an empty submit
async function signIn(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  await userEvent.type(await canvas.findByLabelText(/email/i), "anya@acme.co");
  await userEvent.type(canvas.getByLabelText(/^password/i), "correct-horse");
  await userEvent.click(canvas.getByRole("button", { name: /sign in/i }));
}

export const WrongPassword: Story = {
  render: () => (
    <Harness fetchStub={loginFails(401, "invalid_credentials")}>
      <AuthProvider>
        <Login />
      </AuthProvider>
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await signIn(canvasElement);
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("alert")).toHaveTextContent(
      /do not match an account/i,
    );
    // and crucially: the form is still on screen. the old code would have
    // navigated away, having "signed in" with no session
    await expect(canvas.getByRole("button", { name: /sign in/i })).toBeVisible();
  },
};

export const OrgRequiresSso: Story = {
  render: () => (
    <Harness fetchStub={loginFails(403, "password_login_disabled")}>
      <AuthProvider>
        <Login />
      </AuthProvider>
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await signIn(canvasElement);
    await expect(
      await within(canvasElement).findByRole("alert"),
    ).toHaveTextContent(/single sign-on/i);
  },
};

export const LockedOut: Story = {
  render: () => (
    <Harness
      fetchStub={loginFails(429, "too_many_attempts", { retryAfter: "90" })}
    >
      <AuthProvider>
        <Login />
      </AuthProvider>
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await signIn(canvasElement);
    // the lock is a clock: the wait it carries is rendered, not swallowed
    await expect(
      await within(canvasElement).findByRole("alert"),
    ).toHaveTextContent(/90 seconds/i);
  },
};

export const ControlPlaneUnreachable: Story = {
  render: () => (
    <Harness
      fetchStub={async (input) => {
        if (String(input).includes("/auth/methods")) return json(METHODS);
        throw new TypeError("network error");
      }}
    >
      <AuthProvider>
        <Login />
      </AuthProvider>
    </Harness>
  ),
  play: async ({ canvasElement }) => {
    await signIn(canvasElement);
    await expect(
      await within(canvasElement).findByRole("alert"),
    ).toHaveTextContent(/could not be reached/i);
  },
};
