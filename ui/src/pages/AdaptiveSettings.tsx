import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  fetchAdaptiveRoutingPolicy,
  updateAdaptiveRoutingPolicy,
  MAX_ADAPTIVE_MIN_SAMPLES,
  MAX_ADAPTIVE_WEIGHT,
  MAX_EXPLORATION_RATIO,
  type AdaptiveRoutingPolicyDto,
} from "@/lib/api";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

interface FormState {
  enabled: boolean;
  latencyWeight: string;
  costWeight: string;
  loadWeight: string;
  explorationRatio: string;
  minSamples: string;
}

const fromDto = (dto: AdaptiveRoutingPolicyDto): FormState => ({
  enabled: dto.enabled,
  latencyWeight: String(dto.latency_weight),
  costWeight: String(dto.cost_weight),
  loadWeight: String(dto.load_weight),
  explorationRatio: String(dto.exploration_ratio),
  minSamples: String(dto.min_samples),
});

const WEIGHTS = [
  ["latencyWeight", "Latency", "How strongly observed latency pulls traffic."],
  ["costWeight", "Cost", "How strongly per-token cost pulls traffic."],
  ["loadWeight", "Load", "How strongly in-flight load pushes traffic away."],
] as const;

// mirrors the server's validation so a bad blend is caught before the round
// trip; the server stays the authority and its message is surfaced on reject
function validate(form: FormState): string | null {
  const weights = WEIGHTS.map(([key]) => Number(form[key]));
  if (weights.some((w) => !Number.isFinite(w) || w < 0 || w > MAX_ADAPTIVE_WEIGHT)) {
    return `Each weight must be between 0 and ${MAX_ADAPTIVE_WEIGHT}.`;
  }
  // an all-zero blend does not stop adaptive routing, it turns the strategy
  // into a random balancer — a much less obvious thing to read off a dashboard
  if (weights.every((w) => w <= 0)) {
    return "At least one weight must be positive. Use the kill switch to stop adaptive routing.";
  }
  const ratio = Number(form.explorationRatio);
  if (!Number.isFinite(ratio) || ratio < 0 || ratio > MAX_EXPLORATION_RATIO) {
    return `Exploration ratio must be between 0 and ${MAX_EXPLORATION_RATIO}.`;
  }
  const samples = Number(form.minSamples);
  if (
    !Number.isInteger(samples) ||
    samples < 0 ||
    samples > MAX_ADAPTIVE_MIN_SAMPLES
  ) {
    return `Warm-up samples must be a whole number between 0 and ${MAX_ADAPTIVE_MIN_SAMPLES}.`;
  }
  return null;
}

// global adaptive-routing policy, persisted via /api/v1/adaptive-routing-policy
// (superadmin only). only the ratio between the weights matters — the blend is
// a weighted sum of signals each scored in [0, 1] (#544, #565)
export default function AdaptiveSettings() {
  const queryClient = useQueryClient();
  const policy = useQuery({
    queryKey: ["adaptive-routing-policy"],
    queryFn: fetchAdaptiveRoutingPolicy,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `policy` is the query the user is actually waiting on for this screen
  useScreenReady(!policy.isLoading);
  useErrorState(!!policy.error, "adaptive-settings");

  const [form, setForm] = React.useState<FormState | null>(null);
  const [saved, setSaved] = React.useState(false);
  React.useEffect(() => {
    if (policy.data && form === null) {
      setForm(fromDto(policy.data));
    }
  }, [policy.data, form]);

  const save = useMutation({
    mutationFn: (f: FormState) =>
      updateAdaptiveRoutingPolicy({
        enabled: f.enabled,
        latency_weight: Number(f.latencyWeight),
        cost_weight: Number(f.costWeight),
        load_weight: Number(f.loadWeight),
        exploration_ratio: Number(f.explorationRatio),
        min_samples: Number(f.minSamples),
      }),
    onSuccess: (dto) => {
      queryClient.setQueryData(["adaptive-routing-policy"], dto);
      setForm(fromDto(dto));
      setSaved(true);
    },
  });

  if (policy.isLoading) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-[112px] rounded-[10px]" />
        ))}
      </div>
    );
  }
  if (policy.isError) {
    return (
      <p className="p-[22px] text-sm text-muted-foreground">
        Adaptive routing settings need superadmin access:{" "}
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
  const affected = policy.data?.affected_routes ?? [];
  const total = WEIGHTS.reduce((a, [key]) => a + (Number(form[key]) || 0), 0);

  return (
    <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
      <section className="flex items-start gap-4 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div className="flex-1">
          <span className="text-sm font-medium">Adaptive Routing</span>
          <p className="mt-1 text-sm text-muted-foreground">
            The kill switch. Turning it off leaves every route on the{" "}
            <code className="font-mono text-xs">adaptive</code> strategy serving
            from its static target weights instead.
          </p>
          {/* the blast radius, so the switch is never flipped blind */}
          <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
            {affected.length === 0 ? (
              <span className="text-xs text-muted-foreground">
                No route currently uses the adaptive strategy.
              </span>
            ) : (
              <>
                <span className="text-xs text-muted-foreground">
                  Governs {affected.length}{" "}
                  {affected.length === 1 ? "route" : "routes"}:
                </span>
                {affected.map((model) => (
                  <Badge key={model} tone="outline" className="font-mono">
                    {model}
                  </Badge>
                ))}
              </>
            )}
          </div>
        </div>
        <Switch
          aria-label="Adaptive routing enabled"
          checked={form.enabled}
          onCheckedChange={(enabled) => set({ enabled })}
        />
      </section>

      <section className="flex flex-col gap-3 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Blend Weights</span>
          <p className="mt-1 text-sm text-muted-foreground">
            Each signal is scored in <span className="font-mono text-xs">[0, 1]</span>{" "}
            and combined as a weighted sum, so only the ratio between these
            matters — doubling all three changes nothing.
          </p>
        </div>
        {WEIGHTS.map(([key, label, hint]) => {
          const value = Number(form[key]) || 0;
          const share = total > 0 ? Math.round((value / total) * 100) : 0;
          return (
            <div key={key} className="flex items-center gap-3">
              <div className="flex-1">
                <span className="text-sm">{label}</span>
                <p className="text-xs text-muted-foreground">{hint}</p>
              </div>
              <span className="w-12 text-right font-mono text-xs text-muted-foreground">
                {share}%
              </span>
              <Input
                className="w-[92px]"
                inputMode="decimal"
                aria-label={`${label} weight`}
                value={form[key]}
                onChange={(e) => set({ [key]: e.target.value } as Partial<FormState>)}
              />
            </div>
          );
        })}
      </section>

      <section className="flex flex-col gap-3 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Exploration</span>
          <p className="mt-1 text-sm text-muted-foreground">
            The share of traffic sent off the current best target to keep its
            score fresh, and how many samples a target needs before its score is
            trusted at all. Ratio is capped at {MAX_EXPLORATION_RATIO}.
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="flex-1 text-sm">Exploration ratio</span>
          <Input
            className="w-[92px]"
            inputMode="decimal"
            aria-label="Exploration ratio"
            value={form.explorationRatio}
            onChange={(e) => set({ explorationRatio: e.target.value })}
          />
        </div>
        <div className="flex items-center gap-3">
          <span className="flex-1 text-sm">Warm-up samples</span>
          <Input
            className="w-[92px]"
            inputMode="numeric"
            aria-label="Warm-up samples"
            value={form.minSamples}
            onChange={(e) => set({ minSamples: e.target.value })}
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
            Adaptive routing settings updated.
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
