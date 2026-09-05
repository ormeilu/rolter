import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Gavel, History, Loader2, Megaphone, Play, Trash2 } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { EditorSheet } from "@/components/EditorSheet";
import { ListHeader, ListRow, ListTable, PageBody, Pill, StatusDot } from "@/components/screen";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  ALERT_SIGNALS,
  createAlertChannel,
  createAlertRule,
  deleteAlertChannel,
  deleteAlertRule,
  evaluateAlertRule,
  fetchAlertChannels,
  fetchAlertHistory,
  fetchAlertRules,
  updateAlertChannel,
  updateAlertRule,
  type AlertChannelRow,
  type AlertRuleRow,
} from "@/lib/api";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

const STATE_TONE: Record<string, [string, string]> = {
  ok: ["var(--status-success)", "rgba(22,163,74,.14)"],
  firing: ["var(--status-danger)", "var(--red-tint)"],
  pending: ["var(--status-warning)", "rgba(245,158,11,.14)"],
  unknown: ["var(--text-secondary)", "var(--surface-subtle)"],
};

const stateTone = (state: string) => STATE_TONE[state] ?? STATE_TONE.unknown;

// ---------------------------------------------------------------------------
// channels: webhook destinations alerts are delivered to

export function AlertChannels() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const channels = useQuery({ queryKey: ["alert-channels"], queryFn: fetchAlertChannels, retry: false });

  // UX stream (#805); screen key comes from the enclosing UxScreenProvider
  useScreenReady(!channels.isLoading);
  useErrorState(!!channels.error, "alert-channels");
  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["alert-channels"] });

  const toggle = useMutation({
    mutationFn: (c: AlertChannelRow) =>
      updateAlertChannel(c.id, { name: c.name, endpoint: c.endpoint, enabled: !c.enabled }),
    onSuccess: invalidate,
  });
  const remove = useMutation({ mutationFn: deleteAlertChannel, onSuccess: invalidate });

  const [addOpen, setAddOpen] = React.useState(false);
  // deleting a channel silently strands every rule delivering through it, so
  // the name and the consequence are stated before the request (#1179)
  const [deleteTarget, setDeleteTarget] = React.useState<AlertChannelRow | null>(null);
  const startDelete = (channel: AlertChannelRow) => {
    remove.reset();
    setDeleteTarget(channel);
  };

  return (
    <PageBody>
      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {channels.data?.length ?? 0} channels · webhook destinations for alert delivery
        </span>
        <Button className="ml-auto" onClick={() => setAddOpen(true)}>
          + Add channel
        </Button>
      </div>

      {channels.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {channels.isError && (
        <p className="text-sm text-muted-foreground">
          Alert channels need superadmin access: {(channels.error as Error).message}
        </p>
      )}
      {channels.data && channels.data.length === 0 && (
        <EmptyState
          uxTarget="alert-channels"
          icon={<Megaphone />}
          title={t("pages.alerting.channels.emptyTitle")}
          description={t("pages.alerting.channels.emptyBody")}
          actions={<Button onClick={() => setAddOpen(true)}>{t("pages.alerting.channels.add")}</Button>}
        />
      )}
      <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(340px,1fr))]">
        {(channels.data ?? []).map((c) => (
          <div
            key={c.id}
            className="flex flex-col gap-3 rounded-[10px] border border-[color:var(--border-default)] bg-card p-4"
          >
            <div className="flex items-center gap-2.5">
              <span className="flex h-[34px] w-[34px] flex-none items-center justify-center rounded-lg border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] text-[color:var(--text-secondary)]">
                <Megaphone className="h-4 w-4" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="font-mono text-sm font-semibold">{c.name}</div>
                <div className="truncate text-xs text-muted-foreground">{c.endpoint}</div>
              </div>
              <Switch
                checked={c.enabled}
                disabled={toggle.isPending}
                onCheckedChange={() => toggle.mutate(c)}
              />
            </div>
            <div className="flex items-center gap-2 border-t border-[color:var(--border-subtle)] pt-3">
              <Pill color="var(--text-secondary)" tint="var(--surface-subtle)">
                {c.kind}
              </Pill>
              {c.secret_configured && (
                <Pill color="var(--status-info)" tint="rgba(59,130,246,.14)">
                  secret set
                </Pill>
              )}
              <button
                type="button"
                title="Delete channel"
                aria-label="Delete channel"
                disabled={remove.isPending && remove.variables === c.id}
                onClick={() => startDelete(c)}
                className="ml-auto flex h-[30px] items-center rounded-[6px] border border-[color:var(--border-subtle)] px-2 text-[color:var(--status-danger)] transition-colors hover:bg-[color:var(--red-tint)] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
              >
                {remove.isPending && remove.variables === c.id ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Trash2 className="h-3.5 w-3.5" />
                )}
              </button>
            </div>
          </div>
        ))}
      </div>

      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title={t("pages.alerting.confirm.channelTitle", { name: deleteTarget?.name })}
        description={t("pages.alerting.confirm.channelBody")}
        confirmLabel={t("pages.alerting.confirm.channelConfirm")}
        pending={remove.isPending}
        error={remove.error}
        onConfirm={() =>
          deleteTarget &&
          remove.mutate(deleteTarget.id, { onSuccess: () => setDeleteTarget(null) })
        }
      />

      <AddChannelDialog open={addOpen} onOpenChange={setAddOpen} onDone={invalidate} />
    </PageBody>
  );
}

function AddChannelDialog({
  open,
  onOpenChange,
  onDone,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDone: () => void;
}) {
  const [name, setName] = React.useState("");
  const [endpoint, setEndpoint] = React.useState("");
  const [secret, setSecret] = React.useState("");

  React.useEffect(() => {
    if (open) {
      setName("");
      setEndpoint("");
      setSecret("");
    }
  }, [open]);

  const create = useMutation({
    mutationFn: () =>
      createAlertChannel({
        name,
        endpoint,
        enabled: true,
        ...(secret.trim() ? { managed_secret: secret } : {}),
      }),
    onSuccess: () => {
      onDone();
      onOpenChange(false);
    },
  });

  return (
    <EditorSheet
      open={open}
      onOpenChange={onOpenChange}
      title="Add channel"
      subtitle="Webhook endpoint alerts are POSTed to."
      dirty={Boolean(name || endpoint || secret)}
      errorMessage={create.isError ? (create.error as Error).message : undefined}
      saveLabel="Create"
      canSave={Boolean(name.trim() && endpoint.trim())}
      saving={create.isPending}
      onSave={() => create.mutate()}
    >
      <div className="space-y-3">
        <Field label="Name">
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="ops-slack" />
        </Field>
        <Field label="Endpoint URL">
          <Input
            className="font-mono"
            value={endpoint}
            onChange={(e) => setEndpoint(e.target.value)}
            placeholder="https://hooks.slack.com/services/…"
          />
        </Field>
        <Field label="Bearer secret (optional, write-only)">
          <Input
            type="password"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            placeholder="stored encrypted"
          />
        </Field>
      </div>
    </EditorSheet>
  );
}

// ---------------------------------------------------------------------------
// rules: threshold rules over gateway signals

export function AlertRules() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const rules = useQuery({ queryKey: ["alert-rules"], queryFn: fetchAlertRules, retry: false });

  // UX stream (#805); screen key comes from the enclosing UxScreenProvider
  useScreenReady(!rules.isLoading);
  useErrorState(!!rules.error, "alert-rules");
  const channels = useQuery({ queryKey: ["alert-channels"], queryFn: fetchAlertChannels, retry: false });
  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["alert-rules"] });

  const channelName = (id: string | null) =>
    channels.data?.find((c) => c.id === id)?.name ?? "—";

  const asInput = (r: AlertRuleRow) => ({
    name: r.name,
    signal: r.signal,
    threshold: r.threshold,
    window_secs: r.window_secs,
    channel_id: r.channel_id,
    enabled: r.enabled,
  });
  const toggle = useMutation({
    mutationFn: (r: AlertRuleRow) => updateAlertRule(r.id, { ...asInput(r), enabled: !r.enabled }),
    onSuccess: invalidate,
  });
  const evaluate = useMutation({ mutationFn: evaluateAlertRule, onSuccess: invalidate });
  const remove = useMutation({ mutationFn: deleteAlertRule, onSuccess: invalidate });

  const [addOpen, setAddOpen] = React.useState(false);
  const [deleteTarget, setDeleteTarget] = React.useState<AlertRuleRow | null>(null);
  const startDelete = (rule: AlertRuleRow) => {
    remove.reset();
    setDeleteTarget(rule);
  };

  return (
    <PageBody>
      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {rules.data?.length ?? 0} rules · evaluated every 60s against gateway analytics
        </span>
        <Button className="ml-auto" onClick={() => setAddOpen(true)}>
          + Add rule
        </Button>
      </div>

      {rules.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {rules.isError && (
        <p className="text-sm text-muted-foreground">
          Alert rules need superadmin access: {(rules.error as Error).message}
        </p>
      )}
      {rules.data && rules.data.length === 0 && (
        <EmptyState
          uxTarget="alert-rules"
          icon={<Gavel />}
          title={t("pages.alerting.rules.emptyTitle")}
          description={
            channels.data && channels.data.length === 0
              ? t("pages.alerting.rules.emptyBodyNoChannel")
              : t("pages.alerting.rules.emptyBody")
          }
          actions={<Button onClick={() => setAddOpen(true)}>{t("pages.alerting.rules.add")}</Button>}
        />
      )}
      <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(380px,1fr))]">
        {(rules.data ?? []).map((r) => {
          const tone = stateTone(r.state);
          return (
            <div
              key={r.id}
              className="flex flex-col gap-3 rounded-[10px] border border-[color:var(--border-default)] bg-card p-4"
            >
              <div className="flex items-center gap-2.5">
                <StatusDot color={tone[0]} />
                <span className="min-w-0 truncate font-mono text-sm font-semibold">{r.name}</span>
                <Pill color={tone[0]} tint={tone[1]}>
                  {r.state}
                </Pill>
                <Switch
                  className="ml-auto"
                  checked={r.enabled}
                  disabled={toggle.isPending}
                  onCheckedChange={() => toggle.mutate(r)}
                />
              </div>
              <div className="grid grid-cols-3 gap-2.5">
                <RuleStat label="Signal" value={r.signal} />
                <RuleStat label="Threshold" value={String(r.threshold)} />
                <RuleStat label="Window" value={`${r.window_secs}s`} />
                <RuleStat
                  label="Last value"
                  value={r.last_value === null ? "—" : String(r.last_value)}
                />
                <RuleStat
                  label="Evaluated"
                  value={r.last_evaluated_at ? r.last_evaluated_at.slice(11, 19) : "never"}
                />
                <RuleStat label="Channel" value={channelName(r.channel_id)} />
              </div>
              {r.last_error && (
                <p className="text-xs text-destructive">{r.last_error}</p>
              )}
              <div className="flex items-center gap-2 border-t border-[color:var(--border-subtle)] pt-3">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={evaluate.isPending}
                  onClick={() => evaluate.mutate(r.id)}
                >
                  <Play className="h-3.5 w-3.5" />
                  Evaluate now
                </Button>
                <button
                  type="button"
                  title="Delete rule"
                  aria-label="Delete rule"
                  disabled={remove.isPending && remove.variables === r.id}
                  onClick={() => startDelete(r)}
                  className="ml-auto flex h-[30px] items-center rounded-[6px] border border-[color:var(--border-subtle)] px-2 text-[color:var(--status-danger)] transition-colors hover:bg-[color:var(--red-tint)] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                >
                  {remove.isPending && remove.variables === r.id ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="h-3.5 w-3.5" />
                  )}
                </button>
              </div>
            </div>
          );
        })}
      </div>

      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title={t("pages.alerting.confirm.ruleTitle", { name: deleteTarget?.name })}
        description={t("pages.alerting.confirm.ruleBody")}
        confirmLabel={t("pages.alerting.confirm.ruleConfirm")}
        pending={remove.isPending}
        error={remove.error}
        onConfirm={() =>
          deleteTarget &&
          remove.mutate(deleteTarget.id, { onSuccess: () => setDeleteTarget(null) })
        }
      />

      <AddRuleDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        channels={channels.data ?? []}
        onDone={invalidate}
      />
    </PageBody>
  );
}

function RuleStat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="mb-0.5 text-[0.6875rem] uppercase tracking-[0.05em] text-[color:var(--text-subtle)]">
        {label}
      </div>
      <div className="truncate font-mono text-xs text-[color:var(--text-secondary)]">{value}</div>
    </div>
  );
}

function AddRuleDialog({
  open,
  onOpenChange,
  channels,
  onDone,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  channels: AlertChannelRow[];
  onDone: () => void;
}) {
  const [name, setName] = React.useState("");
  const [signal, setSignal] = React.useState<string>(ALERT_SIGNALS[0]);
  const [threshold, setThreshold] = React.useState("0.05");
  const [windowSecs, setWindowSecs] = React.useState("300");
  const [channelId, setChannelId] = React.useState("");

  React.useEffect(() => {
    if (open) {
      setName("");
      setSignal(ALERT_SIGNALS[0]);
      setThreshold("0.05");
      setWindowSecs("300");
      setChannelId(channels[0]?.id ?? "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const create = useMutation({
    mutationFn: () =>
      createAlertRule({
        name,
        signal,
        threshold: Number(threshold),
        window_secs: Number(windowSecs) || 300,
        channel_id: channelId || null,
        enabled: true,
      }),
    onSuccess: () => {
      onDone();
      onOpenChange(false);
    },
  });

  // the draft is seeded with defaults rather than blanks, so "dirty" is a diff
  // against the seed instead of a plain emptiness check
  const dirty =
    name !== "" ||
    signal !== ALERT_SIGNALS[0] ||
    threshold !== "0.05" ||
    windowSecs !== "300" ||
    channelId !== (channels[0]?.id ?? "");

  return (
    <EditorSheet
      open={open}
      onOpenChange={onOpenChange}
      title="Add rule"
      subtitle="Fires when the signal crosses the threshold within the window."
      dirty={dirty}
      errorMessage={create.isError ? (create.error as Error).message : undefined}
      saveLabel="Create"
      canSave={Boolean(name.trim() && threshold.trim())}
      saving={create.isPending}
      onSave={() => create.mutate()}
    >
      <div className="space-y-3">
        <Field label="Name">
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="high error rate" />
        </Field>
        <Field label="Signal">
          <Select value={signal} onChange={(e) => setSignal(e.target.value)}>
            {ALERT_SIGNALS.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </Select>
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Threshold">
            <Input
              type="number"
              step="any"
              value={threshold}
              onChange={(e) => setThreshold(e.target.value)}
            />
          </Field>
          <Field label="Window (seconds)">
            <Input
              type="number"
              min={30}
              value={windowSecs}
              onChange={(e) => setWindowSecs(e.target.value)}
            />
          </Field>
        </div>
        <Field label="Channel">
          <Select value={channelId} onChange={(e) => setChannelId(e.target.value)}>
            <option value="">none (record only)</option>
            {channels.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </Select>
        </Field>
      </div>
    </EditorSheet>
  );
}

// ---------------------------------------------------------------------------
// history: every notification the evaluator delivered (or failed to)

const HISTORY_GRID = "150px 1.4fr 110px 130px 2fr";

export function AlertHistory() {
  const { t } = useTranslation();
  const history = useQuery({
    queryKey: ["alert-history"],
    queryFn: () => fetchAlertHistory(200),
    retry: false,
  });

  // UX stream (#805); screen key comes from the enclosing UxScreenProvider
  useScreenReady(!history.isLoading);
  useErrorState(!!history.error, "alert-history");
  const rules = useQuery({ queryKey: ["alert-rules"], queryFn: fetchAlertRules, retry: false });
  const ruleName = (id: string) => rules.data?.find((r) => r.id === id)?.name ?? id.slice(0, 8);

  return (
    <PageBody>
      <span className="text-sm text-muted-foreground">
        {history.data?.length ?? 0} notifications · newest first
      </span>
      {history.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {history.isError && (
        <p className="text-sm text-muted-foreground">
          Alert history needs superadmin access: {(history.error as Error).message}
        </p>
      )}
      {history.data && history.data.length === 0 && (
        <EmptyState
          uxTarget="alert-history"
          icon={<History />}
          title={t("pages.alerting.history.emptyTitle")}
          description={t("pages.alerting.history.emptyBody")}
        />
      )}
      {history.data && history.data.length > 0 && (
        <ListTable>
          <ListHeader grid={HISTORY_GRID}>
            <span>Sent</span>
            <span>Rule</span>
            <span>State</span>
            <span>Delivery</span>
            <span>Detail</span>
          </ListHeader>
          {history.data.map((n) => {
            const tone = stateTone(n.state);
            return (
              <ListRow key={n.id} grid={HISTORY_GRID}>
                <span className="font-mono text-xs text-[color:var(--text-secondary)]">
                  {n.sent_at.slice(0, 19).replace("T", " ")}
                </span>
                <span className="truncate font-mono text-xs">{ruleName(n.rule_id)}</span>
                <Pill color={tone[0]} tint={tone[1]}>
                  {n.state}
                </Pill>
                <Pill
                  color={
                    n.delivery_status === "delivered"
                      ? "var(--status-success)"
                      : "var(--status-warning)"
                  }
                  tint="var(--surface-subtle)"
                >
                  {n.delivery_status}
                </Pill>
                <span className="truncate text-xs text-muted-foreground">{n.detail ?? "—"}</span>
              </ListRow>
            );
          })}
        </ListTable>
      )}
    </PageBody>
  );
}
