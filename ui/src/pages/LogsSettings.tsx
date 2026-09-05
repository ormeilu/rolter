import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { LoadError } from "@/components/LoadError";
import { PanelSkeleton } from "@/components/LoadingState";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  fetchLoggingSettings,
  updateLoggingSettings,
  type LoggingSettingsDto,
} from "@/lib/api";
import { errorDetail, useToast } from "@/lib/toast";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

interface FormState {
  samplePercent: string;
  captureEnabled: boolean;
  maxBytes: string;
  redactFields: string;
  models: string;
  virtualKeyIds: string;
  retentionDays: string;
  payloadRetentionHours: string;
}

const splitList = (value: string) =>
  value
    .split(",")
    .map((v) => v.trim())
    .filter(Boolean);

// the API takes a 0..1 fraction; operators think in percent, so the field is
// percent and the conversion happens at the edge
const fromDto = (dto: LoggingSettingsDto): FormState => ({
  samplePercent: String(Math.round(dto.sample_rate * 1000) / 10),
  captureEnabled: dto.payload_capture_enabled,
  maxBytes: String(dto.payload_capture_max_bytes),
  redactFields: dto.payload_capture_redact_fields.join(", "),
  models: dto.payload_capture_models.join(", "),
  virtualKeyIds: dto.payload_capture_virtual_key_ids.join(", "),
  retentionDays: String(dto.retention_days),
  payloadRetentionHours: String(dto.payload_retention_hours),
});

// mirrors the server's validation so a bad value is caught before the round
// trip; the server stays the authority and its message is surfaced on reject
function validate(form: FormState): string | null {
  const percent = Number(form.samplePercent);
  if (!Number.isFinite(percent) || percent < 0 || percent > 100) {
    return "Sample rate must be between 0 and 100 percent.";
  }
  const maxBytes = Number(form.maxBytes);
  if (!Number.isInteger(maxBytes) || maxBytes < 0 || maxBytes > 1_048_576) {
    return "Max payload bytes must be a whole number between 0 and 1048576.";
  }
  const days = Number(form.retentionDays);
  if (!Number.isInteger(days) || days < 1 || days > 3650) {
    return "Retention must be a whole number of days between 1 and 3650.";
  }
  const hours = Number(form.payloadRetentionHours);
  if (!Number.isInteger(hours) || hours < 1 || hours > 8760) {
    return "Payload retention must be a whole number of hours between 1 and 8760.";
  }
  // raw bodies are the sensitive half: keeping them past the metadata they
  // belong to would leak prompt content the operator meant to expire
  if (hours > days * 24) {
    return "Payload retention cannot outlive log retention.";
  }
  return null;
}

// global request-log policy, persisted via /api/v1/logging-settings (superadmin
// only). controls how much traffic is sampled, whether raw payloads are
// captured, what is redacted from them, and how long each is kept (#537)
export default function LogsSettings() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const toast = useToast();
  const settings = useQuery({
    queryKey: ["logging-settings"],
    queryFn: fetchLoggingSettings,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `settings` is the query the user is actually waiting on for this screen
  useScreenReady(!settings.isLoading);
  useErrorState(!!settings.error, "logs-settings");

  const [form, setForm] = React.useState<FormState | null>(null);
  React.useEffect(() => {
    if (settings.data && form === null) {
      setForm(fromDto(settings.data));
    }
  }, [settings.data, form]);

  const save = useMutation({
    mutationFn: (f: FormState) =>
      updateLoggingSettings({
        sample_rate: Number(f.samplePercent) / 100,
        payload_capture_enabled: f.captureEnabled,
        payload_capture_max_bytes: Number(f.maxBytes),
        payload_capture_redact_fields: splitList(f.redactFields),
        payload_capture_models: splitList(f.models),
        payload_capture_virtual_key_ids: splitList(f.virtualKeyIds),
        retention_days: Number(f.retentionDays),
        payload_retention_hours: Number(f.payloadRetentionHours),
      }),
    onSuccess: (dto) => {
      queryClient.setQueryData(["logging-settings"], dto);
      // the cached write alone left every other reader of this key on the
      // value it already had; the refetch is what makes the save stick (#1197)
      void queryClient.invalidateQueries({ queryKey: ["logging-settings"] });
      setForm(fromDto(dto));
      toast.push({
        tone: "success",
        title: t("toast.saved"),
        detail: t("toast.savedDetail", { what: t("errors.resources.logsSettings") }),
      });
    },
    onError: (error) => {
      toast.push({
        tone: "error",
        title: t("toast.saveFailed", { what: t("errors.resources.logsSettings") }),
        detail: errorDetail(error),
      });
    },
  });

  if (settings.isLoading) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        <PanelSkeleton panels={4} height={104} />
      </div>
    );
  }
  if (settings.isError) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        <LoadError
          error={settings.error}
          resource={t("errors.resources.logsSettings")}
          onRetry={() => void settings.refetch()}
        />
      </div>
    );
  }
  if (!form) return null;

  const set = (patch: Partial<FormState>) => {
    setForm((f) => (f ? { ...f, ...patch } : f));
  };
  const localError = validate(form);
  const capture = form.captureEnabled;

  return (
    <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
      <section className="flex flex-col gap-2.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Sample Rate</span>
          <p className="mt-1 text-sm text-muted-foreground">
            Percentage of requests written to the log. Lower it to cut storage on
            high-volume deployments; metrics and budgets are unaffected.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Input
            className="max-w-[120px]"
            inputMode="decimal"
            aria-label="Sample rate percent"
            value={form.samplePercent}
            onChange={(e) => set({ samplePercent: e.target.value })}
          />
          <span className="text-sm text-muted-foreground">% of requests</span>
        </div>
      </section>

      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div className="flex items-start gap-4">
          <div className="min-w-0 flex-1">
            <span className="text-sm font-medium">Capture Request Payloads</span>
            <p className="mt-1 text-sm text-muted-foreground">
              Store raw prompt and completion bodies alongside the log metadata.
              Off by default — payloads carry whatever the caller sent.
            </p>
          </div>
          <Switch
            checked={form.captureEnabled}
            aria-label="Capture request payloads"
            onCheckedChange={(v) => set({ captureEnabled: v })}
          />
        </div>
        {/* a disabled fieldset rather than a dimmed div: the input inside
            already carries `disabled`, and fading a live div drags its label and
            hint below 4.5:1 while telling assistive tech nothing (#1181) */}
        <fieldset
          className="flex min-w-0 flex-col gap-1.5"
          disabled={!capture}
          style={{ opacity: capture ? 1 : 0.55 }}
        >
          <label htmlFor="logs-max-bytes" className="text-xs font-medium text-[color:var(--text-secondary)]">
            Max bytes per payload
          </label>
          <Input
            id="logs-max-bytes"
            className="max-w-[180px]"
            inputMode="numeric"
            disabled={!capture}
            aria-label="Max bytes per payload"
            value={form.maxBytes}
            onChange={(e) => set({ maxBytes: e.target.value })}
          />
          <span className="text-[0.6875rem] text-[color:var(--text-subtle)]">
            Anything longer is truncated. Maximum 1048576 (1 MiB).
          </span>
        </fieldset>
      </section>

      <ListCard
        title="Redacted Fields"
        desc="Comma-separated JSON keys stripped from captured payloads before they are stored. Redaction happens before the write, so a redacted value never reaches storage."
        value={form.redactFields}
        placeholder="authorization, api_key, ssn"
        disabled={!capture}
        onChange={(v) => set({ redactFields: v })}
      />
      <ListCard
        title="Capture Only These Models"
        desc="Comma-separated model names. Leave empty to capture payloads for every model."
        value={form.models}
        placeholder="gpt-4o, claude-sonnet-5"
        disabled={!capture}
        onChange={(v) => set({ models: v })}
      />
      <ListCard
        title="Capture Only These Virtual Keys"
        desc="Comma-separated virtual key UUIDs. Leave empty to capture payloads for every key."
        value={form.virtualKeyIds}
        placeholder="0b7f1e2a-1c3d-4e5f-8a9b-0c1d2e3f4a5b"
        disabled={!capture}
        onChange={(v) => set({ virtualKeyIds: v })}
      />

      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Retention</span>
          <p className="mt-1 text-sm text-muted-foreground">
            How long each half of the log is kept. Payloads are the sensitive
            half, so they cannot outlive the metadata they belong to.
          </p>
        </div>
        <div className="flex flex-wrap gap-4">
          <div className="flex flex-col gap-1.5">
            <label htmlFor="logs-retention-days" className="text-xs font-medium text-[color:var(--text-secondary)]">
              Log metadata (days)
            </label>
            <Input
              id="logs-retention-days"
              className="max-w-[140px]"
              inputMode="numeric"
              aria-label="Retention days"
              value={form.retentionDays}
              onChange={(e) => set({ retentionDays: e.target.value })}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label htmlFor="logs-payload-retention-hours" className="text-xs font-medium text-[color:var(--text-secondary)]">
              Captured payloads (hours)
            </label>
            <Input
              id="logs-payload-retention-hours"
              className="max-w-[140px]"
              inputMode="numeric"
              aria-label="Payload retention hours"
              value={form.payloadRetentionHours}
              onChange={(e) => set({ payloadRetentionHours: e.target.value })}
            />
          </div>
        </div>
      </section>

      <div className="sticky bottom-0 flex items-center justify-end gap-3 border-t border-[color:var(--border-subtle)] bg-background py-3">
        {localError && <span className="text-xs text-[color:var(--status-danger-text)]">{localError}</span>}
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

function ListCard({
  title,
  desc,
  value,
  placeholder,
  disabled,
  onChange,
}: {
  title: string;
  desc: string;
  value: string;
  placeholder: string;
  disabled: boolean;
  onChange: (v: string) => void;
}) {
  return (
    <fieldset
      className="flex min-w-0 flex-col gap-2.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4"
      disabled={disabled}
      style={{ opacity: disabled ? 0.55 : 1 }}
    >
      <div>
        <span className="text-sm font-medium">{title}</span>
        <p className="mt-1 text-sm text-muted-foreground">{desc}</p>
      </div>
      <Textarea
        className="min-h-[64px] font-mono text-xs"
        value={value}
        disabled={disabled}
        aria-label={title}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
    </fieldset>
  );
}
