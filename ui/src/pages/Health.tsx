import { useQuery } from "@tanstack/react-query";
import { HeartPulse } from "lucide-react";
import { useTranslation } from "react-i18next";

import { LoadError } from "@/components/LoadError";
import { CardGridSkeleton } from "@/components/LoadingState";
import { PageBody } from "@/components/screen";
import { EmptyState } from "@/components/ui/empty-state";
import {
  fetchHealthTimeline,
  fetchMttr,
  fetchUptime,
  type TimelineRow,
} from "@/lib/api";
import { useFormat, type Formatters } from "@/lib/i18n/format";
import { cn } from "@/lib/utils";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

const SLA = 0.99;

function pct(fmt: Formatters, v: number): string {
  return fmt.percent(v, 2);
}

function mttrLabel(fmt: Formatters, seconds: number | undefined): string {
  return seconds === undefined ? "—" : fmt.duration(seconds);
}

// one thin bar per time bucket: red if any failure landed in it, else green
function Timeline({ buckets }: { buckets: TimelineRow[] }) {
  const { t } = useTranslation();
  if (buckets.length === 0) {
    return <span className="text-xs text-muted-foreground">{t("pages.health.noEvents")}</span>;
  }
  return (
    <div className="flex h-8 items-end gap-px">
      {buckets.map((b) => {
        const bad = b.errors + b.timeouts;
        const down = bad > 0;
        return (
          <div
            key={b.bucket}
            title={`${b.bucket}: ${b.ok} ok, ${b.errors} error, ${b.timeouts} timeout`}
            className={cn(
              "w-1.5 flex-1 rounded-sm",
              down ? "bg-destructive" : "bg-[color:var(--status-success)]/70",
            )}
            style={{ height: down ? "100%" : "40%" }}
          />
        );
      })}
    </div>
  );
}

export default function Health() {
  const { t } = useTranslation();
  const fmt = useFormat();
  const uptime = useQuery({
    queryKey: ["health-uptime", SLA],
    queryFn: () => fetchUptime(SLA),
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `uptime` is the query the user is actually waiting on for this screen
  useScreenReady(!uptime.isLoading);
  useErrorState(!!uptime.error, "health");
  const mttr = useQuery({ queryKey: ["health-mttr"], queryFn: fetchMttr });
  const timeline = useQuery({
    queryKey: ["health-timeline"],
    queryFn: () => fetchHealthTimeline("hour"),
  });

  const isLoading = uptime.isLoading || mttr.isLoading || timeline.isLoading;
  const error = uptime.error || mttr.error || timeline.error;

  const mttrByTarget = new Map(
    (mttr.data ?? []).map((m) => [`${m.provider}::${m.target_id}`, m]),
  );
  const timelineByTarget = new Map<string, TimelineRow[]>();
  for (const row of timeline.data ?? []) {
    const key = `${row.provider}::${row.target_id}`;
    const list = timelineByTarget.get(key) ?? [];
    list.push(row);
    timelineByTarget.set(key, list);
  }

  return (
    <PageBody className="gap-[18px]">
      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">
          Per-target circuit breakers, uptime, and error-budget burn — last 7 days, SLA target{" "}
          {pct(fmt, SLA)}
        </span>
      </div>
      {isLoading && <CardGridSkeleton cards={4} height={218} min={340} />}
      {error && (
        <LoadError
          error={error}
          resource={t("errors.resources.healthRollups")}
          onRetry={() => {
            uptime.refetch();
            mttr.refetch();
            timeline.refetch();
          }}
        />
      )}
      {!isLoading && !error && (uptime.data?.length ?? 0) === 0 && (
        // health rollups are a by-product of traffic, so there is nothing to
        // create here — the honest CTA is the one that produces an event
        <EmptyState
          uxTarget="health-rollups"
          icon={<HeartPulse />}
          title={t("pages.health.emptyTitle")}
          description={t("pages.health.emptyBody")}
          actions={
            <a
              href="/playground"
              className="text-sm font-medium text-foreground underline decoration-[color:var(--border-strong)] underline-offset-4 transition-colors hover:decoration-current focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {t("pages.health.emptyAction")}
            </a>
          }
        />
      )}
      <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(min(340px,100%),1fr))]">
        {uptime.data?.map((row) => {
          const key = `${row.provider}::${row.target_id}`;
          const breached = row.sla_breached === 1;
          const m = mttrByTarget.get(key);
          // the dot carries a shape, the pill beside it carries a label: two
          // halves of the same status hue, per docs/development/dashboard-theme.md
          const dot = breached ? "var(--status-danger)" : "var(--status-success)";
          const label = breached ? "var(--status-danger-text)" : "var(--status-success-text)";
          return (
            <div
              key={key}
              className="flex flex-col gap-3.5 rounded-[10px] border bg-card p-4"
              style={{
                borderColor: breached
                  ? "color-mix(in srgb, var(--status-danger) 45%, transparent)"
                  : "var(--border-default)",
              }}
            >
              <div className="flex items-center gap-2.5">
                <span className="h-2 w-2 flex-none rounded-full" style={{ background: dot }} />
                <span className="font-mono text-sm font-semibold">{row.target_id}</span>
                <span className="font-mono text-xs text-[color:var(--text-subtle)]">
                  {row.provider}
                </span>
                <span
                  className="ml-auto rounded-[6px] px-2 py-[3px] font-mono text-[0.6875rem] uppercase tracking-[0.05em]"
                  style={{
                    color: label,
                    background: breached ? "rgba(229,57,53,.14)" : "rgba(22,163,74,.14)",
                  }}
                >
                  {breached ? "tripped" : "closed"}
                </span>
              </div>
              <div className="flex items-end gap-3">
                <div className="flex flex-col gap-px">
                  <span
                    className={cn(
                      "font-mono text-2xl font-medium leading-none",
                      breached && "text-[color:var(--status-danger-text)]",
                    )}
                  >
                    {pct(fmt, row.uptime)}
                  </span>
                  <span className="text-[0.6875rem] uppercase tracking-[0.06em] text-[color:var(--text-subtle)]">
                    uptime · {row.events} events
                  </span>
                </div>
                <div className="min-w-0 flex-1">
                  <Timeline buckets={timelineByTarget.get(key) ?? []} />
                </div>
              </div>
              <div className="grid grid-cols-1 gap-2.5 border-t sm:grid-cols-3 border-[color:var(--border-subtle)] pt-3">
                <div>
                  <div className="mb-0.5 text-[0.6875rem] uppercase tracking-[0.05em] text-[color:var(--text-subtle)]">
                    Failures
                  </div>
                  <div className="font-mono text-sm text-[color:var(--text-secondary)]">
                    {row.errors + row.timeouts}
                  </div>
                </div>
                <div>
                  <div className="mb-0.5 text-[0.6875rem] uppercase tracking-[0.05em] text-[color:var(--text-subtle)]">
                    MTTR
                  </div>
                  <div className="font-mono text-sm text-[color:var(--text-secondary)]">
                    {mttrLabel(fmt, m?.mttr_seconds)}
                  </div>
                </div>
                <div>
                  <div className="mb-0.5 text-[0.6875rem] uppercase tracking-[0.05em] text-[color:var(--text-subtle)]">
                    Incidents
                  </div>
                  <div className="font-mono text-sm text-[color:var(--text-secondary)]">
                    {m?.incidents ?? "—"}
                  </div>
                </div>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">error budget burn</span>
                <span
                  className="ml-auto font-mono text-xs"
                  style={{
                    color:
                      row.error_budget_burn > 1
                        ? "var(--status-danger-text)"
                        : "var(--text-secondary)",
                  }}
                >
                  {fmt.percent(row.error_budget_burn, 0)}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </PageBody>
  );
}
