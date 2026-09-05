import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

/**
 * Lists config entries the control plane is not serving to gateways (#926).
 *
 * A malformed provider is dropped from the snapshot rather than freezing the
 * whole fleet's config. That is the right blast radius, but it makes the
 * failure quiet: every gateway keeps serving its last good config, so nothing
 * looks broken until someone wonders why a change never took effect. This is
 * the thing that says so.
 */
export function UnservedConfigNotice({ problems }: { problems: string[] }) {
  const { t } = useTranslation();
  if (problems.length === 0) return null;
  return (
    <div className="space-y-2 rounded-lg border border-[color:var(--border-subtle)] bg-[color:var(--red-tint)] p-4">
      <div className="flex items-center gap-2">
        <AlertTriangle
          aria-hidden
          className="h-4 w-4 flex-none text-[color:var(--status-warning-text)]"
        />
        <p className="text-sm font-medium text-foreground">
          {t("providers.unserved.title", { count: problems.length })}
        </p>
      </div>
      <ul className="space-y-1 pl-6">
        {problems.map((problem) => (
          <li
            key={problem}
            className="list-disc text-sm text-muted-foreground marker:text-[color:var(--status-warning-text)]"
          >
            {problem}
          </li>
        ))}
      </ul>
    </div>
  );
}
