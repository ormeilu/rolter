import { ArrowRight, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  acceptInvitation,
  previewInvitation,
  type InvitationPreview,
} from "@/lib/api";
import { useAuth } from "@/lib/auth";

// the invitee has no account yet, so this screen renders outside the signed-in
// shell. the token in the url is the only credential it has.
export default function AcceptInvite({ token }: { token: string }) {
  const { signIn } = useAuth();
  const [invite, setInvite] = useState<InvitationPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pw, setPw] = useState("");
  const [confirm, setConfirm] = useState("");
  const [pending, setPending] = useState(false);

  useEffect(() => {
    let live = true;
    void previewInvitation(token)
      .then((p) => live && setInvite(p))
      .catch(
        () =>
          live &&
          setError(
            "This invitation link is not valid. It may have been used, revoked, or expired — ask whoever invited you for a new one.",
          ),
      );
    return () => {
      live = false;
    };
  }, [token]);

  const submit = async () => {
    setPending(true);
    setError(null);
    try {
      const res = await acceptInvitation(token, pw);
      signIn(res.user.email, res.token);
      // land on the dashboard rather than back on a spent link
      window.history.replaceState(null, "", "/dashboard");
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
    }
  };

  const mismatch = confirm.length > 0 && confirm !== pw;
  const ready = pw.length >= 8 && !mismatch && !pending;

  return (
    <div className="flex min-h-screen items-center justify-center bg-[color:var(--surface-app)] p-4">
      <div className="w-[400px] max-w-full overflow-hidden rounded-xl border bg-background shadow-2xl">
        <div className="vyshivka-rule" />
        <div className="flex flex-col gap-6 p-8">
          <div className="flex items-center gap-3">
            <img src="/logo-mark.svg" alt="" className="h-10 w-10" />
            <span className="font-mono text-[22px] font-semibold tracking-tight">
              rolter<span className="text-[color:var(--red-folk)]">.</span>
            </span>
          </div>

          {invite == null ? (
            <p className="text-sm text-muted-foreground">
              {error ?? "Checking your invitation..."}
            </p>
          ) : (
            <>
              <div className="flex flex-col gap-1">
                <h1 className="text-xl font-semibold">
                  Join {invite.org_name}
                </h1>
                <p className="text-sm text-muted-foreground">
                  You were invited as <strong>{invite.email}</strong> with the{" "}
                  <strong>{invite.role}</strong> role. Choose a password — only
                  you will know it.
                </p>
              </div>
              <form
                className="flex flex-col gap-4"
                onSubmit={(e) => {
                  e.preventDefault();
                  void submit();
                }}
              >
                <label className="flex flex-col gap-1.5 text-sm">
                  <span className="text-muted-foreground">
                    Password (at least 8 characters)
                  </span>
                  <Input
                    type="password"
                    value={pw}
                    onChange={(e) => setPw(e.target.value)}
                    autoComplete="new-password"
                  />
                </label>
                <label className="flex flex-col gap-1.5 text-sm">
                  <span className="text-muted-foreground">
                    Confirm password
                  </span>
                  <Input
                    type="password"
                    value={confirm}
                    onChange={(e) => setConfirm(e.target.value)}
                    autoComplete="new-password"
                  />
                </label>
                {mismatch && (
                  <p className="text-xs text-destructive">
                    The two passwords do not match.
                  </p>
                )}
                {error != null && (
                  <p className="text-xs text-destructive">{error}</p>
                )}
                <Button
                  type="submit"
                  disabled={!ready}
                  className="w-full bg-brand text-white hover:bg-brand-hover"
                >
                  {pending ? (
                    <>
                      Creating account...{" "}
                      <Loader2 className="h-4 w-4 animate-spin" />
                    </>
                  ) : (
                    <>
                      Accept invitation <ArrowRight className="h-4 w-4" />
                    </>
                  )}
                </Button>
              </form>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
