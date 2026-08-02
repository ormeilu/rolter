import type * as React from "react";
import { ShieldCheck } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";

export function GuardrailLoading() {
  return (
    <div className="grid gap-3 md:grid-cols-2" aria-label="Loading guardrails">
      {[0, 1, 2].map((item) => (
        <Skeleton key={item} width="100%" height={172} radius={10} />
      ))}
    </div>
  );
}

export function GuardrailEmpty({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action: React.ReactNode;
}) {
  return (
    <EmptyState
      icon={<ShieldCheck />}
      title={title}
      description={description}
      actions={action}
    />
  );
}

export function GuardrailError({ message, retry }: { message: string; retry: () => void }) {
  return (
    <div className="rounded-[10px] border border-[color:var(--status-danger)]/30 bg-[color:var(--status-danger)]/5 p-5">
      <p className="text-sm font-medium text-[color:var(--status-danger)]">
        Guardrail registry unavailable
      </p>
      <p className="mt-1 text-sm text-muted-foreground">{message}</p>
      <Button className="mt-4" variant="outline" onClick={retry}>
        Try again
      </Button>
    </div>
  );
}

export function PolicyCard({
  title,
  description,
  enabled,
  badges,
  details,
  actions,
}: {
  title: string;
  description: string;
  enabled: boolean;
  badges: React.ReactNode;
  details: React.ReactNode;
  actions: React.ReactNode;
}) {
  return (
    <article className="flex min-h-[172px] flex-col rounded-[10px] border border-[color:var(--border-subtle)] bg-[color:var(--surface-raised)] p-4 transition-colors hover:border-[color:var(--border-default)]">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-sm font-semibold">{title}</h2>
            <Badge tone={enabled ? "success" : "neutral"} dot>
              {enabled ? "enforced" : "paused"}
            </Badge>
          </div>
          <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">{description}</p>
        </div>
      </div>
      <div className="mt-3 flex flex-wrap gap-1.5">{badges}</div>
      <div className="mt-3 text-xs text-[color:var(--text-subtle)]">{details}</div>
      <div className="mt-auto flex justify-end gap-2 pt-4">{actions}</div>
    </article>
  );
}
