import { Check, Copy } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

/**
 * Small icon button that copies `value` to the clipboard and briefly shows a
 * checkmark. Used to make `provider-slug/model` addresses one-click copyable.
 */
export function CopyButton({
  value,
  /** overrides the shared `common.copy` label; already-translated when passed */
  label,
  className,
}: {
  value: string;
  label?: string;
  className?: string;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = React.useState(false);
  const copyLabel = label ?? t("common.copy");
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1200);
    } catch {
      // clipboard unavailable (e.g. insecure context): leave state untouched
    }
  };

  return (
    <Button
      type="button"
      size="sm"
      variant="ghost"
      className={className}
      onClick={copy}
      aria-label={`${copyLabel}: ${value}`}
      title={copied ? t("common.copied") : copyLabel}
    >
      {copied ? (
        <Check className="h-3.5 w-3.5" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </Button>
  );
}
