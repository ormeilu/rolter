import { useQuery } from "@tanstack/react-query";
import { Activity, Network } from "lucide-react";

import { PageBody } from "@/components/screen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { StatCard } from "@/components/ui/stat-card";
import {
  fetchAdaptiveRoutingTelemetry,
  type AdaptiveDecisionCountsDto,
  type AdaptiveNodeTelemetryDto,
  type AdaptiveRouteTelemetryDto,
  type AdaptiveTargetTelemetryDto,
} from "@/lib/api";

const count = new Intl.NumberFormat();
const decimal = new Intl.NumberFormat(undefined, { maximumFractionDigits: 3 });

function totalDecisions(decisions: AdaptiveDecisionCountsDto): number {
  return decisions.blend + decisions.exploration + decisions.fallback;
}

function percent(value: number, total: number): string {
  return total > 0 ? `${Math.round((value / total) * 100)}%` : "0%";
}

function formatLatency(value?: number): string {
  if (!value || value <= 0) return "No samples";
  return `${decimal.format(value)} ms`;
}

function formatCost(value?: number): string {
  if (!value || value <= 0) return "Unknown";
  return decimal.format(value);
}

function policySummary(node: AdaptiveNodeTelemetryDto): string {
  const policy = node.policy;
  const weights = [
    policy.latency_weight != null ? `${decimal.format(policy.latency_weight)} latency` : null,
    policy.cost_weight != null ? `${decimal.format(policy.cost_weight)} cost` : null,
    policy.load_weight != null ? `${decimal.format(policy.load_weight)} load` : null,
  ].filter(Boolean);
  const details = weights.length > 0 ? weights.join(" / ") : "policy unavailable";
  const summary = policy.min_samples == null
    ? details
    : `${details} · ${count.format(policy.min_samples)} warm-up samples`;
  return policy.enabled === false ? `disabled · ${summary}` : summary;
}

function routeIsDisabled(route: AdaptiveRouteTelemetryDto): boolean {
  return route.nodes.length > 0 && route.nodes.every((node) => node.policy.enabled === false);
}

function nodeState(node: AdaptiveNodeTelemetryDto): "DISABLED" | "ROUTING" | "WARMING" {
  if (node.policy.enabled === false) return "DISABLED";
  return node.engaged ? "ROUTING" : "WARMING";
}

function targetName(target: AdaptiveTargetTelemetryDto): string {
  const provider = target.provider?.trim() || `target ${target.target + 1}`;
  const model = target.upstream_model?.trim();
  return model ? `${provider} / ${model}` : provider;
}

export default function AdaptiveDashboard() {
  const telemetry = useQuery({
    queryKey: ["adaptive-routing-telemetry"],
    queryFn: fetchAdaptiveRoutingTelemetry,
    retry: false,
    refetchInterval: 15_000,
  });

  if (telemetry.isLoading) {
    return (
      <PageBody aria-label="Loading adaptive routing telemetry">
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          {[0, 1, 2, 3].map((item) => (
            <Skeleton key={item} height={92} radius={10} />
          ))}
        </div>
        {[0, 1].map((item) => (
          <Skeleton key={item} height={224} radius={10} />
        ))}
      </PageBody>
    );
  }

  if (telemetry.isError) {
    return (
      <PageBody>
        <div
          role="alert"
          className="flex max-w-[65ch] flex-col items-start gap-3 rounded-[10px] border border-[color:var(--border-subtle)] p-4"
        >
          <div>
            <p className="text-sm font-medium text-foreground">
              Unable to load adaptive routing telemetry
            </p>
            <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
              Check your superadmin access and connection, then try again.
            </p>
            <p className="mt-1 font-mono text-xs text-muted-foreground">
              {telemetry.error.message}
            </p>
          </div>
          <Button variant="outline" onClick={() => void telemetry.refetch()}>
            Try again
          </Button>
        </div>
      </PageBody>
    );
  }

  const view = telemetry.data;
  const routes = view?.routes ?? [];
  const nodes = new Set(routes.flatMap((route) => route.nodes.map((node) => node.node_id)));
  const observed = routes.reduce(
    (routeTotal, route) =>
      routeTotal + route.nodes.reduce((nodeTotal, node) => nodeTotal + node.observed, 0),
    0,
  );
  const engaged = routes.filter((route) => route.engaged).length;

  return (
    <PageBody>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p className="max-w-[65ch] text-sm leading-relaxed text-muted-foreground">
          Live scores and decision modes reported by gateways using the adaptive strategy.
          Counts reset when a gateway rebuilds its routing snapshot.
        </p>
        {view && (
          <p className="text-xs tabular-nums text-muted-foreground">
            <time dateTime={view.generated_at} title={new Date(view.generated_at).toLocaleString()}>
              Updated {new Date(view.generated_at).toLocaleTimeString()}
            </time>
            {` · reports expire after ${count.format(view.fresh_window_secs)} s`}
          </p>
        )}
      </div>

      {routes.length === 0 ? (
        <EmptyState
          icon={<Activity aria-hidden="true" />}
          title="No fresh adaptive routing telemetry"
          description={`A gateway appears here after it serves a route on the adaptive strategy and reports to the control plane. Reports older than ${view?.fresh_window_secs ?? 60} seconds are hidden.`}
          actions={
            <a
              href="/routing-rules"
              className="text-sm font-medium text-foreground underline decoration-[color:var(--border-strong)] underline-offset-4 transition-colors hover:decoration-current focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              Review routing rules
            </a>
          }
        />
      ) : (
        <>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
            <StatCard label="Adaptive routes" value={count.format(routes.length)} />
            <StatCard label="Blend active" value={count.format(engaged)} />
            <StatCard label="Reporting gateways" value={count.format(nodes.size)} />
            <StatCard label="Observed picks" value={count.format(observed)} />
          </div>

          <div className="flex flex-col gap-4">
            {routes.map((route) => (
              <section
                key={route.model}
                aria-label={`Adaptive route ${route.model}`}
                className="overflow-hidden rounded-[10px] border border-[color:var(--border-subtle)]"
              >
                <div className="flex flex-wrap items-center justify-between gap-3 bg-[color:var(--surface-subtle)] px-4 py-3">
                  <div className="min-w-0">
                    <h2 className="break-words font-mono text-sm font-medium text-foreground">
                      {route.model}
                    </h2>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {route.nodes.length} reporting {route.nodes.length === 1 ? "gateway" : "gateways"}
                    </p>
                  </div>
                  <Badge
                    tone={route.engaged ? "success" : routeIsDisabled(route) ? "outline" : "warning"}
                    dot={!routeIsDisabled(route)}
                  >
                    {route.engaged
                      ? "BLEND ACTIVE"
                      : routeIsDisabled(route)
                        ? "DISABLED"
                        : "FALLBACK / WARMING"}
                  </Badge>
                </div>

                <div className="flex flex-col gap-3 p-3">
                  {route.nodes.map((node, nodeIndex) => {
                    const decisionTotal = totalDecisions(node.decisions);
                    return (
                      <details
                        key={node.node_id}
                        open={nodeIndex === 0}
                        className="group rounded-lg border border-[color:var(--border-subtle)] bg-[color:var(--surface-card)]"
                      >
                        <summary className="flex cursor-pointer list-none flex-wrap items-center justify-between gap-3 rounded-lg px-4 py-3 transition-colors hover:bg-[color:var(--surface-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
                          <span className="flex min-w-0 items-center gap-2">
                            <Network className="h-4 w-4 flex-none text-muted-foreground" aria-hidden="true" />
                            <span className="break-all font-mono text-sm font-medium">
                              {node.node_id}
                            </span>
                          </span>
                          <span className="flex flex-wrap items-center justify-end gap-2">
                            <Badge tone={node.engaged ? "success" : "outline"}>
                              {nodeState(node)}
                            </Badge>
                            <span className="text-xs tabular-nums text-muted-foreground">
                              {count.format(decisionTotal)} decisions
                            </span>
                          </span>
                        </summary>

                        <div className="flex flex-col gap-4 border-t border-[color:var(--border-subtle)] p-4">
                          <div className="flex flex-wrap items-start justify-between gap-3">
                            <p className="max-w-[65ch] text-xs leading-relaxed text-muted-foreground">
                              {policySummary(node)}
                            </p>
                            <time
                              dateTime={node.reported_at}
                              title={new Date(node.reported_at).toLocaleString()}
                              className="text-xs tabular-nums text-muted-foreground"
                            >
                              Reported {new Date(node.reported_at).toLocaleTimeString()}
                            </time>
                          </div>

                          <div aria-label={`${node.node_id} decision modes`}>
                            <div
                              className="flex h-2 overflow-hidden rounded-full bg-[color:var(--surface-subtle)]"
                              aria-hidden="true"
                            >
                              <span
                                className="bg-[color:var(--status-success)]"
                                style={{ width: percent(node.decisions.blend, decisionTotal) }}
                              />
                              <span
                                className="bg-[color:var(--status-info)]"
                                style={{ width: percent(node.decisions.exploration, decisionTotal) }}
                              />
                              <span
                                className="bg-[color:var(--text-subtle)]"
                                style={{ width: percent(node.decisions.fallback, decisionTotal) }}
                              />
                            </div>
                            <dl className="mt-2 grid grid-cols-1 gap-2 text-xs sm:grid-cols-3">
                              {([
                                ["Blend", node.decisions.blend],
                                ["Exploration", node.decisions.exploration],
                                ["Fallback", node.decisions.fallback],
                              ] as const).map(([label, value]) => (
                                <div key={label} className="flex items-baseline justify-between gap-2">
                                  <dt className="text-muted-foreground">{label}</dt>
                                  <dd className="font-mono tabular-nums text-foreground">
                                    {count.format(value)} · {percent(value, decisionTotal)}
                                  </dd>
                                </div>
                              ))}
                            </dl>
                          </div>

                          {node.targets.length === 0 ? (
                            <p className="text-sm text-muted-foreground">
                              No target signals were included in this report.
                            </p>
                          ) : (
                            <div className="overflow-x-auto rounded-lg border border-[color:var(--border-subtle)]">
                              <table className="w-full min-w-[760px] border-collapse text-sm">
                                <caption className="sr-only">
                                  Target signals reported by {node.node_id}
                                </caption>
                                <thead className="bg-[color:var(--surface-subtle)] text-left text-[0.6875rem] uppercase tracking-[0.07em] text-muted-foreground">
                                  <tr>
                                    <th scope="col" className="px-3 py-2 font-medium">Upstream</th>
                                    <th scope="col" className="px-3 py-2 text-right font-medium">Blend score</th>
                                    <th scope="col" className="px-3 py-2 text-right font-medium">Latency</th>
                                    <th scope="col" className="px-3 py-2 text-right font-medium">Cost / Mtok</th>
                                    <th scope="col" className="px-3 py-2 text-right font-medium">In flight</th>
                                    <th scope="col" className="px-3 py-2 text-right font-medium">Samples</th>
                                  </tr>
                                </thead>
                                <tbody>
                                  {node.targets.map((target) => (
                                    <tr
                                      key={target.target}
                                      className="border-t border-[color:var(--border-subtle)]"
                                    >
                                      <th scope="row" className="break-words px-3 py-2.5 text-left font-mono text-xs font-medium">
                                        {targetName(target)}
                                      </th>
                                      <td className="px-3 py-2.5 text-right font-mono text-xs tabular-nums">
                                        {target.score == null ? "—" : decimal.format(target.score)}
                                      </td>
                                      <td className="px-3 py-2.5 text-right font-mono text-xs tabular-nums">
                                        {formatLatency(target.latency_ms)}
                                      </td>
                                      <td className="px-3 py-2.5 text-right font-mono text-xs tabular-nums">
                                        {formatCost(target.cost_per_mtok)}
                                      </td>
                                      <td className="px-3 py-2.5 text-right font-mono text-xs tabular-nums">
                                        {count.format(target.in_flight ?? 0)}
                                      </td>
                                      <td className="px-3 py-2.5 text-right font-mono text-xs tabular-nums">
                                        {count.format(target.samples ?? 0)}
                                      </td>
                                    </tr>
                                  ))}
                                </tbody>
                              </table>
                            </div>
                          )}
                        </div>
                      </details>
                    );
                  })}
                </div>
              </section>
            ))}
          </div>
        </>
      )}
    </PageBody>
  );
}
