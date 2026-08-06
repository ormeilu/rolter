import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  fetchCompatibilityPolicy,
  updateCompatibilityPolicy,
  type CompatibilityPolicyDto,
} from "@/lib/api";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

interface FormState {
  anthropicVersion: string;
  defaultMaxTokens: string;
}

const fromDto = (dto: CompatibilityPolicyDto): FormState => ({
  anthropicVersion: dto.anthropic_version,
  defaultMaxTokens: String(dto.default_max_tokens),
});

// a dated Anthropic release string like 2023-06-01. the gateway forwards this
// verbatim as anthropic-version, so a malformed value fails every Anthropic
// call at the edge with an opaque upstream error
const DATED_RELEASE = /^\d{4}-\d{2}-\d{2}$/;

// mirrors the server's validation so a bad value is caught before the round
// trip; the server stays the authority and its message is surfaced on reject
function validate(form: FormState): string | null {
  if (!DATED_RELEASE.test(form.anthropicVersion.trim())) {
    return "Anthropic version must be a dated release like 2023-06-01.";
  }
  const tokens = Number(form.defaultMaxTokens);
  if (!Number.isInteger(tokens) || tokens < 1 || tokens > 1_000_000) {
    return "Default max tokens must be a whole number between 1 and 1000000.";
  }
  return null;
}

// cross-dialect compatibility policy, persisted via /api/v1/compatibility-policy
// (superadmin only). these are the values applied when a request is translated
// between the OpenAI and Anthropic wire formats (#546)
export default function Compatibility() {
  const queryClient = useQueryClient();
  const policy = useQuery({
    queryKey: ["compatibility-policy"],
    queryFn: fetchCompatibilityPolicy,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `policy` is the query the user is actually waiting on for this screen
  useScreenReady(!policy.isLoading);
  useErrorState(!!policy.error, "compatibility");

  const [form, setForm] = React.useState<FormState | null>(null);
  const [saved, setSaved] = React.useState(false);
  React.useEffect(() => {
    if (policy.data && form === null) {
      setForm(fromDto(policy.data));
    }
  }, [policy.data, form]);

  const save = useMutation({
    mutationFn: (f: FormState) =>
      updateCompatibilityPolicy({
        anthropic_version: f.anthropicVersion.trim(),
        default_max_tokens: Number(f.defaultMaxTokens),
      }),
    onSuccess: (dto) => {
      queryClient.setQueryData(["compatibility-policy"], dto);
      setForm(fromDto(dto));
      setSaved(true);
    },
  });

  if (policy.isLoading) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        {[0, 1].map((i) => (
          <Skeleton key={i} className="h-[112px] rounded-[10px]" />
        ))}
      </div>
    );
  }
  if (policy.isError) {
    return (
      <p className="p-[22px] text-sm text-muted-foreground">
        Compatibility settings need superadmin access:{" "}
        {(policy.error as Error).message}
      </p>
    );
  }
  if (!form) return null;

  const set = (patch: Partial<FormState>) => {
    setForm((f) => (f ? { ...f, ...patch } : f));
    setSaved(false);
  };
  const localError = validate(form);
  // the server owns this list, so the screen warns without knowing which
  // fields need a restart
  const restartRequired = policy.data?.restart_required ?? [];

  return (
    <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
      <section className="flex flex-col gap-2.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Anthropic API Version</span>
          <p className="mt-1 text-sm text-muted-foreground">
            Sent as the <code className="font-mono text-xs">anthropic-version</code>{" "}
            header on every request Rolter translates into the Anthropic dialect.
            Forwarded verbatim, so it must be a dated release.
          </p>
        </div>
        <Input
          className="max-w-[200px] font-mono text-xs"
          aria-label="Anthropic API version"
          placeholder="2023-06-01"
          value={form.anthropicVersion}
          onChange={(e) => set({ anthropicVersion: e.target.value })}
        />
      </section>

      <section className="flex flex-col gap-2.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Default Max Tokens</span>
          <p className="mt-1 text-sm text-muted-foreground">
            Anthropic requires <code className="font-mono text-xs">max_tokens</code>{" "}
            while OpenAI treats it as optional. This value fills the gap when an
            OpenAI-shaped request that omits it is translated.
          </p>
        </div>
        <Input
          className="max-w-[200px]"
          inputMode="numeric"
          aria-label="Default max tokens"
          value={form.defaultMaxTokens}
          onChange={(e) => set({ defaultMaxTokens: e.target.value })}
        />
      </section>

      {restartRequired.length > 0 && (
        <section className="flex items-start gap-3 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
          <Badge tone="warning">RESTART</Badge>
          <p className="text-sm text-muted-foreground">
            These fields only take effect after a gateway restart:{" "}
            <span className="font-mono text-xs">{restartRequired.join(", ")}</span>
          </p>
        </section>
      )}

      <div className="sticky bottom-0 flex items-center justify-end gap-3 border-t border-[color:var(--border-subtle)] bg-background py-3">
        {localError && <span className="text-xs text-destructive">{localError}</span>}
        {!localError && save.isError && (
          <span className="text-xs text-destructive">
            {(save.error as Error).message}
          </span>
        )}
        {saved && (
          <span className="text-xs text-[color:var(--status-success)]">
            Compatibility settings updated.
          </span>
        )}
        <Button
          disabled={save.isPending || localError !== null}
          onClick={() => save.mutate(form)}
        >
          {save.isPending ? "Saving…" : "Save Changes"}
        </Button>
      </div>
    </div>
  );
}
