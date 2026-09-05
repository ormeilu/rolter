import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { LoadError } from "@/components/LoadError";
import { PanelSkeleton } from "@/components/LoadingState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  fetchModelDefaults,
  updateModelDefaults,
  type ModelDefaultsDto,
} from "@/lib/api";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

// every field is optional, so the form keeps raw strings and an empty string
// means "leave this to the provider" rather than "send zero"
interface FormState {
  enabled: boolean;
  defaultModel: string;
  temperature: string;
  topP: string;
  maxTokens: string;
}

const text = (value: string | null) => value ?? "";
const num = (value: number | null) => (value === null ? "" : String(value));

const fromDto = (dto: ModelDefaultsDto): FormState => ({
  enabled: dto.enabled,
  defaultModel: text(dto.default_model),
  temperature: num(dto.default_temperature),
  topP: num(dto.default_top_p),
  maxTokens: num(dto.default_max_tokens),
});

const blank = (value: string) => value.trim() === "";
const parse = (value: string) => (blank(value) ? null : Number(value));

const inRange = (value: string, min: number, max: number, integer = false) => {
  if (blank(value)) return true;
  const n = Number(value);
  if (!Number.isFinite(n)) return false;
  if (integer && !Number.isInteger(n)) return false;
  return n >= min && n <= max;
};

// mirrors the server's validation so a bad value is caught before the round
// trip; the server stays the authority and its message is surfaced on reject
function validate(form: FormState): string | null {
  if (form.defaultModel.length > 256) {
    return "Default model must be at most 256 characters.";
  }
  if (!inRange(form.temperature, 0, 2)) return "Temperature must be between 0 and 2.";
  if (!inRange(form.topP, 0, 1)) return "Top-p must be between 0 and 1.";
  if (!inRange(form.maxTokens, 1, 1_000_000, true)) {
    return "Max tokens must be a whole number between 1 and 1000000.";
  }
  return null;
}

const hasAnyDefault = (form: FormState) =>
  !blank(form.defaultModel) ||
  !blank(form.temperature) ||
  !blank(form.topP) ||
  !blank(form.maxTokens);

// deployment-wide inference defaults, persisted via /api/v1/model-defaults
// (superadmin only). they only ever fill a gap: a parameter the client sent is
// never overwritten, which is what makes this safe to turn on mid-flight
export default function ModelSettings() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const defaults = useQuery({
    queryKey: ["model-defaults"],
    queryFn: fetchModelDefaults,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `defaults` is the query the user is actually waiting on for this screen
  useScreenReady(!defaults.isLoading);
  useErrorState(!!defaults.error, "model-settings");

  const [form, setForm] = React.useState<FormState | null>(null);
  const [saved, setSaved] = React.useState(false);
  React.useEffect(() => {
    if (defaults.data && form === null) {
      setForm(fromDto(defaults.data));
    }
  }, [defaults.data, form]);

  const save = useMutation({
    mutationFn: (f: FormState) =>
      updateModelDefaults({
        enabled: f.enabled,
        default_model: blank(f.defaultModel) ? null : f.defaultModel.trim(),
        default_temperature: parse(f.temperature),
        default_top_p: parse(f.topP),
        default_max_tokens: parse(f.maxTokens),
      }),
    onSuccess: (dto) => {
      queryClient.setQueryData(["model-defaults"], dto);
      setForm(fromDto(dto));
      setSaved(true);
    },
  });

  if (defaults.isLoading) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        <PanelSkeleton panels={2} height={152} />
      </div>
    );
  }
  if (defaults.isError) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        <LoadError
          error={defaults.error}
          resource={t("errors.resources.modelSettings")}
          onRetry={() => void defaults.refetch()}
        />
      </div>
    );
  }
  if (!form) return null;

  const set = (patch: Partial<FormState>) => {
    setForm((f) => (f ? { ...f, ...patch } : f));
    setSaved(false);
  };
  const localError = validate(form);
  const active = form.enabled && hasAnyDefault(form);

  return (
    <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div className="flex items-start gap-4">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium">Apply defaults</span>
              <Badge tone={active ? "success" : "neutral"} className="font-mono text-[10px] uppercase">
                {active ? "active" : "inactive"}
              </Badge>
            </div>
            <p className="mt-1 text-sm text-muted-foreground">
              Fill in the parameters below when a request omits them. A value the
              client sent is never overwritten, so turning this on cannot change
              a request that was already explicit. With it off, request bodies
              reach the provider untouched.
            </p>
          </div>
          <Switch
            checked={form.enabled}
            aria-label="Apply defaults"
            onCheckedChange={(v) => set({ enabled: v })}
          />
        </div>
        {form.enabled && !hasAnyDefault(form) && (
          <p className="text-xs text-[color:var(--text-subtle)]">
            No defaults are set yet, so nothing is applied. Fill in at least one
            field below.
          </p>
        )}
      </section>

      <Card
        title="Sampling"
        desc="Applied to chat, messages and responses requests. Embeddings, audio and image endpoints take no sampling parameters and are left alone."
        dimmed={!form.enabled}
      >
        <Field
          label="Temperature"
          hint="0 – 2"
          value={form.temperature}
          disabled={!form.enabled}
          onChange={(v) => set({ temperature: v })}
        />
        <Field
          label="Top-p"
          hint="0 – 1"
          value={form.topP}
          disabled={!form.enabled}
          onChange={(v) => set({ topP: v })}
        />
        <Field
          label="Max tokens"
          hint="Completion length cap"
          value={form.maxTokens}
          disabled={!form.enabled}
          onChange={(v) => set({ maxTokens: v })}
        />
      </Card>

      <Card
        title="Model"
        desc="Used when a request arrives without a model. Leave this empty to keep rejecting such requests instead of silently picking one for the caller."
        dimmed={!form.enabled}
      >
        <Field
          label="Default model"
          hint="A routed model or group-slug/model address"
          wide
          value={form.defaultModel}
          disabled={!form.enabled}
          onChange={(v) => set({ defaultModel: v })}
        />
      </Card>

      <p className="text-xs text-muted-foreground">
        Upstream timeouts and the retry budget are deployment-wide and live under{" "}
        <span className="text-[color:var(--text-secondary)]">
          Settings &rsaquo; Performance Tuning
        </span>
        .
      </p>

      <div className="sticky bottom-0 flex items-center justify-end gap-3 border-t border-[color:var(--border-subtle)] bg-background py-3">
        {localError && <span className="text-xs text-destructive">{localError}</span>}
        {!localError && save.isError && (
          <span className="text-xs text-destructive">{(save.error as Error).message}</span>
        )}
        {saved && (
          <span className="text-xs text-[color:var(--status-success)]">
            Model defaults updated.
          </span>
        )}
        <Button disabled={save.isPending || localError !== null} onClick={() => save.mutate(form)}>
          {save.isPending ? "Saving…" : "Save Changes"}
        </Button>
      </div>
    </div>
  );
}

function Card({
  title,
  desc,
  dimmed = false,
  children,
}: {
  title: string;
  desc: string;
  dimmed?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
      <div>
        <span className="text-sm font-medium">{title}</span>
        <p className="mt-1 text-sm text-muted-foreground">{desc}</p>
      </div>
      <div className="flex flex-wrap gap-4" style={{ opacity: dimmed ? 0.55 : 1 }}>
        {children}
      </div>
    </section>
  );
}

function Field({
  label,
  hint,
  value,
  disabled = false,
  wide = false,
  onChange,
}: {
  label: string;
  hint: string;
  value: string;
  disabled?: boolean;
  wide?: boolean;
  onChange: (v: string) => void;
}) {
  const id = React.useId();
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-xs font-medium text-[color:var(--text-secondary)]">
        {label}
      </label>
      <Input
        id={id}
        className={wide ? "min-w-[320px]" : "max-w-[160px]"}
        placeholder="provider default"
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
      />
      <span className="text-[0.6875rem] text-[color:var(--text-subtle)]">{hint}</span>
    </div>
  );
}
