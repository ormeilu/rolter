import { useTranslation } from "react-i18next";

import { strategyHintKey } from "@/lib/strategies";

/**
 * The caveat attached to a balancing strategy, or nothing (#897).
 *
 * Some strategies are not simply "pick one and it works": two need a telemetry
 * source configured on the member providers and degrade quietly to least-load
 * without it, and one is governed by a deployment-wide policy rather than by
 * this control. Selecting them without knowing that produces a route that looks
 * configured and does not behave as chosen — which is exactly the silence #897
 * was about, in a different place.
 */
export function StrategyHint({ strategy }: { strategy: string }) {
  const { t } = useTranslation();
  const key = strategyHintKey(strategy);
  if (!key) return null;
  return (
    <p className="mt-1.5 text-xs text-[color:var(--status-warning-text)]" role="note">
      {t(key)}
    </p>
  );
}
