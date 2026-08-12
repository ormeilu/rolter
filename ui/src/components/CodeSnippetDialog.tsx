import { Code2 } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { CopyButton } from "@/components/CopyButton";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Select } from "@/components/ui/select";
import {
  renderSnippet,
  SNIPPET_LANGS,
  type SnippetLang,
  type SnippetRequest,
} from "@/lib/snippets";

const LABELS: Record<SnippetLang, string> = {
  curl: "curl",
  python: "Python",
  javascript: "JavaScript",
};

/**
 * "Copy as code" for a Playground request.
 *
 * The Playground proves a route works; the next thing an operator does is wire
 * that route into an application. The useful output of a successful call is
 * therefore the code that reproduces it, not the reply — and rolter's whole
 * pitch is that the code is just the OpenAI SDK with a different `base_url`,
 * which is far more convincing shown than described.
 */
export function CodeSnippetDialog({
  open,
  onOpenChange,
  request,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  request: SnippetRequest;
}) {
  const { t } = useTranslation();
  const [lang, setLang] = React.useState<SnippetLang>("curl");

  // window is read at render rather than module load so the snippet follows
  // whatever host the dashboard is actually being served from
  const snippet = React.useMemo(
    () =>
      renderSnippet(
        lang,
        request,
        typeof window === "undefined" ? "" : window.location.origin,
      ),
    [lang, request],
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogHeader>
        <DialogTitle>{t("playground.copyAsCode")}</DialogTitle>
        <DialogDescription>{t("playground.copyAsCodeHint")}</DialogDescription>
      </DialogHeader>

      <div className="flex items-center gap-2">
        <Select
          value={lang}
          onChange={(e) => setLang(e.target.value as SnippetLang)}
          aria-label={t("playground.language")}
        >
          {SNIPPET_LANGS.map((l) => (
            <option key={l} value={l}>
              {LABELS[l]}
            </option>
          ))}
        </Select>
        <CopyButton value={snippet} className="ml-auto" />
      </div>

      <pre className="max-h-[340px] overflow-auto rounded-md border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] p-3 text-xs leading-relaxed">
        <code>{snippet}</code>
      </pre>

      <DialogFooter>
        <Button variant="ghost" onClick={() => onOpenChange(false)}>
          {t("common.close")}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

/** The trigger, so a caller only has to own the request it describes. */
export function CopyAsCodeButton({ request }: { request: SnippetRequest }) {
  const { t } = useTranslation();
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button
        size="sm"
        variant="ghost"
        className="h-8"
        onClick={() => setOpen(true)}
        aria-label={t("playground.copyAsCode")}
        title={t("playground.copyAsCode")}
      >
        <Code2 className="h-3.5 w-3.5" />
      </Button>
      {open && (
        <CodeSnippetDialog open={open} onOpenChange={setOpen} request={request} />
      )}
    </>
  );
}
