import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  fetchRuntimePolicy,
  updateRuntimePolicy,
  BACKPRESSURE_POLICIES,
  type BackpressurePolicy,
  type RuntimePolicyDto,
} from "@/lib/api";

interface FormState {
  retryMaxRetries: string;
  retryBaseMs: string;
  retryMaxMs: string;
  timeoutConnectS: string;
  timeoutRequestS: string;
  queueEnabled: boolean;
  queueCapacity: string;
  queueWorkers: string;
  queueBackpressure: BackpressurePolicy;
  queueBlockMs: string;
}

const fromDto = (dto: RuntimePolicyDto): FormState => ({
  retryMaxRetries: String(dto.retry_max_retries),
  retryBaseMs: String(dto.retry_base_ms),
  retryMaxMs: String(dto.retry_max_ms),
  timeoutConnectS: String(dto.timeout_connect_s),
  timeoutRequestS: String(dto.timeout_request_s),
  queueEnabled: dto.queue_enabled,
  queueCapacity: String(dto.queue_capacity),
  queueWorkers: String(dto.queue_workers),
  queueBackpressure: dto.queue_backpressure,
  queueBlockMs: String(dto.queue_block_ms),
});

const BACKPRESSURE_COPY: Record<BackpressurePolicy, string> = {
  drop: "Drop — shed the request immediately with an overload response.",
  block: "Block — wait for capacity until the block timeout expires.",
  error: "Error — return a structured queue_full error immediately.",
};

const inRange = (value: string, min: number, max: number) => {
  const n = Number(value);
  return Number.isInteger(n) && n >= min && n <= max;
};

// mirrors the server's validation so a bad value is caught before the round
// trip; the server stays the authority and its message is surfaced on reject
function validate(form: FormState): string | null {
  if (!inRange(form.retryMaxRetries, 0, 10)) return "Max retries must be between 0 and 10.";
  if (!inRange(form.retryBaseMs, 0, 60_000)) return "Retry base must be between 0 and 60000 ms.";
  if (!inRange(form.retryMaxMs, 0, 600_000)) return "Retry cap must be between 0 and 600000 ms.";
  if (Number(form.retryMaxMs) < Number(form.retryBaseMs)) {
    return "Retry cap cannot be lower than the retry base.";
  }
  if (!inRange(form.timeoutConnectS, 0, 300)) return "Connect timeout must be between 0 and 300 s.";
  if (!inRange(form.timeoutRequestS, 0, 3_600)) {
    return "Request timeout must be between 0 and 3600 s.";
  }
  if (!inRange(form.queueCapacity, 1, 100_000)) {
    return "Queue capacity must be between 1 and 100000.";
  }
  if (!inRange(form.queueWorkers, 1, 2_048)) return "Queue workers must be between 1 and 2048.";
  if (!inRange(form.queueBlockMs, 0, 120_000)) {
    return "Block timeout must be between 0 and 120000 ms.";
  }
  // blocking with a zero timeout would park callers forever
  if (form.queueBackpressure === "block" && Number(form.queueBlockMs) === 0) {
    return "Block backpressure needs a non-zero block timeout.";
  }
  return null;
}

// global runtime policy, persisted via /api/v1/runtime-policy (superadmin only).
// covers upstream retries, timeouts and the bounded admission queue every
// provider gets its own instance of
export default function Performance() {
  const queryClient = useQueryClient();
  const policy = useQuery({
    queryKey: ["runtime-policy"],
    queryFn: fetchRuntimePolicy,
    retry: false,
  });

  const [form, setForm] = React.useState<FormState | null>(null);
  const [saved, setSaved] = React.useState(false);
  React.useEffect(() => {
    if (policy.data && form === null) {
      setForm(fromDto(policy.data));
    }
  }, [policy.data, form]);

  const save = useMutation({
    mutationFn: (f: FormState) =>
      updateRuntimePolicy({
        retry_max_retries: Number(f.retryMaxRetries),
        retry_base_ms: Number(f.retryBaseMs),
        retry_max_ms: Number(f.retryMaxMs),
        timeout_connect_s: Number(f.timeoutConnectS),
        timeout_request_s: Number(f.timeoutRequestS),
        queue_enabled: f.queueEnabled,
        queue_capacity: Number(f.queueCapacity),
        queue_workers: Number(f.queueWorkers),
        queue_backpressure: f.queueBackpressure,
        queue_block_ms: Number(f.queueBlockMs),
      }),
    onSuccess: (dto) => {
      queryClient.setQueryData(["runtime-policy"], dto);
      setForm(fromDto(dto));
      setSaved(true);
    },
  });

  if (policy.isLoading) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-[132px] rounded-[10px]" />
        ))}
      </div>
    );
  }
  if (policy.isError) {
    return (
      <p className="p-[22px] text-sm text-muted-foreground">
        Performance tuning needs superadmin access:{" "}
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
  const queue = form.queueEnabled;

  return (
    <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
      <Card
        title="Retries"
        desc="Upstream retry budget for a single client request. Backoff grows exponentially from the base up to the cap."
      >
        <NumberField
          label="Max retries"
          value={form.retryMaxRetries}
          onChange={(v) => set({ retryMaxRetries: v })}
        />
        <NumberField
          label="Base backoff (ms)"
          value={form.retryBaseMs}
          onChange={(v) => set({ retryBaseMs: v })}
        />
        <NumberField
          label="Backoff cap (ms)"
          value={form.retryMaxMs}
          onChange={(v) => set({ retryMaxMs: v })}
        />
      </Card>

      <Card
        title="Timeouts"
        desc="How long the gateway waits on an upstream. Zero disables the timeout — streaming responses are bounded by the request timeout too."
      >
        <NumberField
          label="Connect (s)"
          value={form.timeoutConnectS}
          onChange={(v) => set({ timeoutConnectS: v })}
        />
        <NumberField
          label="Request (s)"
          value={form.timeoutRequestS}
          onChange={(v) => set({ timeoutRequestS: v })}
        />
      </Card>

      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div className="flex items-start gap-4">
          <div className="min-w-0 flex-1">
            <span className="text-sm font-medium">Admission Queue</span>
            <p className="mt-1 text-sm text-muted-foreground">
              Each provider gets its own bounded queue and worker set, so a slow
              provider cannot consume the admission capacity of healthy ones.
            </p>
          </div>
          <Switch
            checked={form.queueEnabled}
            aria-label="Admission queue"
            onCheckedChange={(v) => set({ queueEnabled: v })}
          />
        </div>
        <div
          className="flex flex-wrap gap-4"
          style={{ opacity: queue ? 1 : 0.55 }}
        >
          <NumberField
            label="Capacity"
            value={form.queueCapacity}
            disabled={!queue}
            onChange={(v) => set({ queueCapacity: v })}
          />
          <NumberField
            label="Workers"
            value={form.queueWorkers}
            disabled={!queue}
            onChange={(v) => set({ queueWorkers: v })}
          />
          <div className="flex min-w-[200px] flex-col gap-1.5">
            <label className="text-xs font-medium text-[color:var(--text-secondary)]">
              When the queue is full
            </label>
            <Select
              value={form.queueBackpressure}
              disabled={!queue}
              aria-label="When the queue is full"
              onChange={(e) =>
                set({ queueBackpressure: e.target.value as BackpressurePolicy })
              }
            >
              {BACKPRESSURE_POLICIES.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </Select>
            <span className="text-[0.6875rem] text-[color:var(--text-subtle)]">
              {BACKPRESSURE_COPY[form.queueBackpressure]}
            </span>
          </div>
          <NumberField
            label="Block timeout (ms)"
            value={form.queueBlockMs}
            disabled={!queue || form.queueBackpressure !== "block"}
            onChange={(v) => set({ queueBlockMs: v })}
          />
        </div>
      </section>

      <div className="sticky bottom-0 flex items-center justify-end gap-3 border-t border-[color:var(--border-subtle)] bg-background py-3">
        {localError && <span className="text-xs text-destructive">{localError}</span>}
        {!localError && save.isError && (
          <span className="text-xs text-destructive">
            {(save.error as Error).message}
          </span>
        )}
        {saved && (
          <span className="text-xs text-[color:var(--status-success)]">
            Runtime policy updated.
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

function Card({
  title,
  desc,
  children,
}: {
  title: string;
  desc: string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
      <div>
        <span className="text-sm font-medium">{title}</span>
        <p className="mt-1 text-sm text-muted-foreground">{desc}</p>
      </div>
      <div className="flex flex-wrap gap-4">{children}</div>
    </section>
  );
}

function NumberField({
  label,
  value,
  disabled = false,
  onChange,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  onChange: (v: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-xs font-medium text-[color:var(--text-secondary)]">
        {label}
      </label>
      <Input
        className="max-w-[160px]"
        inputMode="numeric"
        aria-label={label}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}
