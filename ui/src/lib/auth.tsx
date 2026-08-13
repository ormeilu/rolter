import * as React from "react";

// client session state. a real login (POST /api/v1/auth/login) stores an opaque
// bearer token that api.ts attaches to every request, which is what the
// self-service /me/* endpoints need. login stays optional: against an open-mode
// deployment (no accounts) the Login screen falls back to an email-only gate
// with no token, and the admin dashboard keeps working exactly as before.
interface AuthState {
  email: string | null;
  token: string | null;
  signIn: (email: string, token?: string | null) => void;
  signOut: () => void;
}

const EMAIL_KEY = "rolter.session.email";
const TOKEN_KEY = "rolter.session.token";
const AuthContext = React.createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [email, setEmail] = React.useState<string | null>(() =>
    localStorage.getItem(EMAIL_KEY),
  );
  const [token, setToken] = React.useState<string | null>(() =>
    localStorage.getItem(TOKEN_KEY),
  );

  const value: AuthState = React.useMemo(
    () => ({
      email,
      token,
      signIn: (e, t = null) => {
        localStorage.setItem(EMAIL_KEY, e);
        setEmail(e);
        if (t) {
          localStorage.setItem(TOKEN_KEY, t);
          setToken(t);
        } else {
          localStorage.removeItem(TOKEN_KEY);
          setToken(null);
        }
      },
      signOut: () => {
        localStorage.removeItem(EMAIL_KEY);
        localStorage.removeItem(TOKEN_KEY);
        setEmail(null);
        setToken(null);
      },
    }),
    [email, token],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  const ctx = React.useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}

/**
 * The session if there is a provider above, `null` if there is not.
 *
 * For components that *offer* something session-related without depending on
 * it — [`LoadError`](../components/LoadError.tsx) shows a "sign in again"
 * button only when signing in is possible. Throwing there would turn a
 * component whose whole job is to explain a failure into a second failure.
 */
export function useOptionalAuth(): AuthState | null {
  return React.useContext(AuthContext);
}
