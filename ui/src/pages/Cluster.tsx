import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Network } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { PageBody } from "@/components/screen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, type TableColumn } from "@/components/ui/table";
import {
  fetchClusterNodes,
  forgetClusterNode,
  setClusterNodeDrain,
  type ClusterNodeRow,
} from "@/lib/api";
import { useFormat } from "@/lib/i18n/format";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

// nodes fall out of the liveness window in under a minute, so the inventory is
// only useful if it refreshes on its own
const REFETCH_MS = 15_000;

// the two health axes the server reports separately: a node can be polling but
// lagging the newest snapshot, or converged but no longer polling at all
function StateBadges({ node }: { node: ClusterNodeRow }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Badge dot tone={node.live ? "success" : "danger"}>
        {node.live ? "LIVE" : "STALE"}
      </Badge>
      <Badge tone={node.converged ? "neutral" : "warning"}>
        {node.converged ? "CONVERGED" : "LAGGING"}
      </Badge>
      {node.desired_state === "draining" && <Badge tone="accent">DRAINING</Badge>}
    </div>
  );
}

// cluster inventory backed by /api/v1/cluster/nodes (superadmin only). nodes
// are never enrolled here — every gateway identifies itself on its snapshot
// poll, so this screen only reads the inventory and drains or forgets a node
// (#543, #568)
export default function Cluster() {
  const { t } = useTranslation();
  const fmt = useFormat();
  const queryClient = useQueryClient();
  const nodes = useQuery({
    queryKey: ["cluster-nodes"],
    queryFn: fetchClusterNodes,
    refetchInterval: REFETCH_MS,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `nodes` is the query the user is actually waiting on for this screen
  useScreenReady(!nodes.isLoading);
  useErrorState(!!nodes.error, "cluster");
  // one clock for every relative timestamp, so the rows do not drift apart
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["cluster-nodes"] });

  const drain = useMutation({
    mutationFn: (v: { id: string; draining: boolean }) =>
      setClusterNodeDrain(v.id, v.draining),
    onSuccess: invalidate,
  });
  const forget = useMutation({
    mutationFn: (id: string) => forgetClusterNode(id),
    onSuccess: invalidate,
  });
  const actionError = (drain.error ?? forget.error) as Error | undefined;

  // forgetting drops the node's history from the inventory, so it is confirmed
  // by id — and the dialog repeats the one thing that surprises people, that a
  // still-running node comes straight back (#1179)
  const [forgetTarget, setForgetTarget] = React.useState<ClusterNodeRow | null>(
    null,
  );
  const startForget = (node: ClusterNodeRow) => {
    forget.reset();
    setForgetTarget(node);
  };

  const columns: TableColumn<ClusterNodeRow & Record<string, unknown>>[] = [
    { key: "id", header: "Node", mono: true },
    {
      key: "role",
      header: "Role",
      render: (_v, row) => <Badge tone="outline">{row.role}</Badge>,
    },
    { key: "build_version", header: "Build", mono: true },
    {
      key: "config_version",
      header: "Config",
      align: "right",
      mono: true,
      render: (_v, row) => `v${row.config_version}`,
    },
    {
      key: "state",
      header: "State",
      render: (_v, row) => <StateBadges node={row} />,
    },
    {
      key: "last_seen_at",
      header: "Last seen",
      align: "right",
      render: (_v, row) => (
        <span title={fmt.dateTime(row.last_seen_at)}>
          {fmt.relative(row.last_seen_at, now)}
        </span>
      ),
    },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (_v, row) => {
        const draining = row.desired_state === "draining";
        return (
          <div className="flex justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={drain.isPending}
              onClick={() => drain.mutate({ id: row.id, draining: !draining })}
            >
              {drain.isPending && drain.variables?.id === row.id && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {draining ? "Return to service" : "Drain"}
            </Button>
            {/* a node that is still running reappears on its next poll, so
                forgetting is only meaningful once it has gone stale */}
            <Button
              variant="ghost"
              size="sm"
              disabled={row.live || forget.isPending}
              title={
                row.live
                  ? "node is still polling; it would reappear on its next snapshot poll"
                  : undefined
              }
              onClick={() => startForget(row)}
            >
              {forget.isPending && forget.variables === row.id && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              Forget
            </Button>
          </div>
        );
      },
    },
  ];

  if (nodes.isLoading) {
    return (
      <PageBody>
        <Skeleton className="h-9 w-[260px] rounded-md" />
        <Skeleton className="h-[220px] rounded-lg" />
      </PageBody>
    );
  }
  if (nodes.isError) {
    return (
      <p className="p-[22px] text-sm text-muted-foreground">
        Cluster inventory needs superadmin access:{" "}
        {(nodes.error as Error).message}
      </p>
    );
  }

  const rows = nodes.data ?? [];
  const live = rows.filter((n) => n.live).length;
  const lagging = rows.filter((n) => n.live && !n.converged).length;

  return (
    <PageBody>
      <div className="flex flex-wrap items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {rows.length} nodes · {live} live
          {lagging > 0 && ` · ${lagging} still applying the newest config`}
        </span>
        {actionError && !forgetTarget && (
          <span className="text-xs text-destructive">{actionError.message}</span>
        )}
      </div>

      <ConfirmDialog
        open={!!forgetTarget}
        onOpenChange={(open) => !open && setForgetTarget(null)}
        title={t("pages.cluster.confirm.title", { id: forgetTarget?.id })}
        description={t("pages.cluster.confirm.body")}
        confirmLabel={t("pages.cluster.confirm.confirm")}
        pending={forget.isPending}
        error={forget.error}
        onConfirm={() =>
          forgetTarget &&
          forget.mutate(forgetTarget.id, { onSuccess: () => setForgetTarget(null) })
        }
      />

      {rows.length === 0 ? (
        <EmptyState uxTarget="cluster-nodes"
          icon={<Network />}
          title="No nodes have reported in"
          description="Nodes appear here once a gateway identifies itself on its snapshot poll. A single-node deployment that sends no identity headers stays out of the inventory."
        />
      ) : (
        <Table
          columns={columns}
          data={rows as (ClusterNodeRow & Record<string, unknown>)[]}
          rowKey="id"
        />
      )}
    </PageBody>
  );
}
