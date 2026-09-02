import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, Plus, RotateCw, Trash2 } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import {
  DEFAULT_KEY_TTL_DAYS,
  KeyExpiryField,
  KeyModelsField,
  KeyNameField,
  KeyReachSummary,
  keyNameProblem,
  parseModels,
  ttlToDays,
} from "@/components/KeyMintFields";
import { LoadError } from "@/components/LoadError";
import { EditorSheet } from "@/components/EditorSheet";
import { PageBody } from "@/components/screen";
import { SelfServiceUnavailable } from "@/components/SelfServiceUnavailable";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tag } from "@/components/ui/tag";
import {
  AnalyticsUnavailableError,
  deleteMyKey,
  fetchMyKeys,
  isOpenModeNoSession,
  fetchMyUsage,
  mintMyKey,
  rotateMyKey,
  type MintedKey,
  type MyUsageRow,
  type OwnedKeyRow,
} from "@/lib/api";
import { useFormat } from "@/lib/i18n/format";
import { useScope } from "@/lib/scope";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

// end-user self-service panel (ROL-224): view/rotate/delete the virtual keys you
// personally minted and see your own usage/spend. no admin role required — the
// backend scopes everything to the logged-in account.
export default function Account() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const scope = useScope();

  const keys = useQuery({ queryKey: ["my-keys"], queryFn: fetchMyKeys });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;

  // `keys` is the query the user is actually waiting on for this screen

  useScreenReady(!keys.isLoading);

  useErrorState(!!keys.error, "account");
  const usage = useQuery({
    queryKey: ["my-usage"],
    queryFn: () => fetchMyUsage(),
    retry: false,
  });

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["my-keys"] });
    queryClient.invalidateQueries({ queryKey: ["my-usage"] });
  };

  const removeKey = useMutation({
    mutationFn: (id: string) => deleteMyKey(id),
    onSuccess: invalidate,
  });

  const [mintOpen, setMintOpen] = React.useState(false);
  const [minted, setMinted] = React.useState<MintedKey | null>(null);
  const [deleteTarget, setDeleteTarget] = React.useState<OwnedKeyRow | null>(
    null,
  );

  // usage rows keyed by virtual_key_id, for merging into each key card
  const usageByKey = React.useMemo(() => {
    const map = new Map<string, MyUsageRow>();
    for (const row of usage.data ?? []) map.set(row.virtual_key_id, row);
    return map;
  }, [usage.data]);

  const usageUnavailable = usage.error instanceof AnalyticsUnavailableError;
  const selfServiceUnavailable = isOpenModeNoSession(keys.error);

  return (
    <PageBody>
      {/* the screen sits under "Account" in a product that also has provider
          keys and an admin token; say which credential this one is (#943) */}
      <p className="text-sm text-muted-foreground">
        {t("account.keys.explainer")}
      </p>
      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {t("account.keys.summary", { count: keys.data?.length ?? 0 })}
        </span>
        <Button
          className="ml-auto"
          onClick={() => setMintOpen(true)}
          // minting posts to /me/*, which 401s for the same reason the list
          // did; offering the button would just move the dead end one click
          // later (#942)
          disabled={!scope.projectId || selfServiceUnavailable}
          title={
            scope.projectId ? undefined : t("account.keys.selectProject")
          }
        >
          <Plus className="h-4 w-4" />
          {t("account.keys.generate")}
        </Button>
      </div>

      {keys.isLoading && (
        <p className="text-sm text-muted-foreground">
          {t("account.keys.loading")}
        </p>
      )}
      {/* open mode is not a failed request, it is a screen this deployment
          cannot serve at all — saying so beats a red line about loading (#942) */}
      {selfServiceUnavailable && <SelfServiceUnavailable />}
      {keys.error && !selfServiceUnavailable && (
        <LoadError
          error={keys.error}
          resource={t("errors.resources.yourKeys")}
          onRetry={() => keys.refetch()}
        />
      )}
      {!keys.isLoading && keys.data?.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t("account.keys.empty")}
        </p>
      )}

      <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(320px,1fr))]">
        {keys.data?.map((key) => (
          <KeyCard
            key={key.id}
            keyRow={key}
            usage={usageByKey.get(key.id)}
            usageUnavailable={usageUnavailable}
            onRotated={(m) => {
              invalidate();
              setMinted(m);
            }}
            onDelete={() => setDeleteTarget(key)}
          />
        ))}
      </div>

      {scope.projectId && (
        <MintKeyDialog
          open={mintOpen}
          onOpenChange={setMintOpen}
          projectId={scope.projectId}
          projectLabel={
            scope.projects.find((p) => p.id === scope.projectId)?.name
          }
          onMinted={(m) => {
            invalidate();
            setMinted(m);
          }}
        />
      )}

      <Dialog
        open={!!deleteTarget}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
      >
        <DialogHeader>
          <DialogTitle>{t("account.keys.delete.title")}</DialogTitle>
          <DialogDescription>
            <span className="font-mono">{deleteTarget?.key_prefix}…</span>{" "}
            {t("account.keys.delete.body")}
          </DialogDescription>
        </DialogHeader>
        {removeKey.isError && (
          <p className="text-xs text-destructive">
            {(removeKey.error as Error).message}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setDeleteTarget(null)}>
            {t("account.keys.delete.cancel")}
          </Button>
          <Button
            variant="destructive"
            disabled={removeKey.isPending}
            onClick={() => {
              if (!deleteTarget) return;
              removeKey.mutate(deleteTarget.id, {
                onSuccess: () => setDeleteTarget(null),
              });
            }}
          >
            {t("account.keys.delete.confirm")}
          </Button>
        </DialogFooter>
      </Dialog>

      <RevealedKeyDialog
        minted={minted}
        onOpenChange={(open) => !open && setMinted(null)}
      />
    </PageBody>
  );
}

function KeyCard({
  keyRow,
  usage,
  usageUnavailable,
  onRotated,
  onDelete,
}: {
  keyRow: OwnedKeyRow;
  usage?: { requests: number | string; cost_usd: number | string };
  usageUnavailable: boolean;
  onRotated: (m: MintedKey) => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const format = useFormat();
  const rotate = useMutation({
    mutationFn: () => rotateMyKey(keyRow.id),
    onSuccess: onRotated,
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center justify-between gap-2">
          <span className="truncate">
            {keyRow.name ?? t("account.keys.card.unnamed")}
          </span>
          <Badge tone={keyRow.disabled ? "danger" : "success"}>
            {keyRow.disabled
              ? t("account.keys.card.disabled")
              : t("account.keys.card.active")}
          </Badge>
        </CardTitle>
        <CardDescription className="font-mono">
          {keyRow.key_prefix}…
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-xs text-muted-foreground">
          {keyRow.org_name} / {keyRow.project_name}
        </p>
        <div className="flex flex-wrap gap-1.5">
          {keyRow.models.length ? (
            keyRow.models.map((m) => <Tag key={m}>{m}</Tag>)
          ) : (
            <Badge tone="neutral">{t("account.keys.card.allModels")}</Badge>
          )}
        </div>
        <div className="text-xs text-muted-foreground">
          {usageUnavailable ? (
            <span>{t("account.keys.card.usageUnavailable")}</span>
          ) : usage ? (
            <span>
              {t("account.keys.card.usage", {
                requests: format.number(Number(usage.requests)),
                cost: format.currency(Number(usage.cost_usd)),
              })}
            </span>
          ) : (
            <span>{t("account.keys.card.noUsage")}</span>
          )}
        </div>
        <div className="flex items-center justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={rotate.isPending}
            onClick={() => rotate.mutate()}
            title={t("account.keys.card.rotateHint")}
          >
            <RotateCw className="h-3.5 w-3.5" />
            {t("account.keys.card.rotate")}
          </Button>
          <Button
            size="sm"
            variant="destructive"
            onClick={onDelete}
            title={t("account.keys.card.deleteHint")}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function MintKeyDialog({
  open,
  onOpenChange,
  projectId,
  projectLabel,
  onMinted,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projectId: string;
  projectLabel?: string;
  onMinted: (m: MintedKey) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = React.useState("");
  const [modelsText, setModelsText] = React.useState("");
  const [ttl, setTtl] = React.useState(String(DEFAULT_KEY_TTL_DAYS));

  React.useEffect(() => {
    if (open) {
      setName("");
      setModelsText("");
      setTtl(String(DEFAULT_KEY_TTL_DAYS));
    }
  }, [open]);

  const models = React.useMemo(() => parseModels(modelsText), [modelsText]);
  const project = projectLabel ?? t("account.keys.mint.currentProject");

  const mint = useMutation({
    mutationFn: () =>
      mintMyKey(projectId, {
        name: name.trim(),
        models,
        expires_in_days: ttlToDays(ttl),
      }),
    onSuccess: (m) => {
      onOpenChange(false);
      onMinted(m);
    },
  });

  return (
    <EditorSheet
      open={open}
      onOpenChange={onOpenChange}
      title={t("account.keys.mint.title")}
      subtitle={t("account.keys.mint.subtitle", { project })}
      dirty={Boolean(name || modelsText)}
      errorMessage={mint.isError ? (mint.error as Error).message : undefined}
      saveLabel={t("account.keys.mint.save")}
      canSave={keyNameProblem(name) === null}
      saving={mint.isPending}
      onSave={() => mint.mutate()}
    >
      <div className="space-y-3">
        <KeyNameField value={name} onChange={setName} />
        <KeyExpiryField value={ttl} onChange={setTtl} />
        <KeyModelsField value={modelsText} onChange={setModelsText} />
        <KeyReachSummary project={project} models={models} ttl={ttl} />
      </div>
    </EditorSheet>
  );
}

// shows the plaintext secret exactly once after mint/rotate; discarded on close
function RevealedKeyDialog({
  minted,
  onOpenChange,
}: {
  minted: MintedKey | null;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = React.useState(false);

  React.useEffect(() => {
    if (minted) setCopied(false);
  }, [minted]);

  const copy = async () => {
    if (!minted) return;
    try {
      await navigator.clipboard.writeText(minted.key);
      setCopied(true);
    } catch {
      // clipboard unavailable — user can still select/copy the text manually
    }
  };

  return (
    <Dialog open={!!minted} onOpenChange={onOpenChange}>
      <DialogHeader>
        <DialogTitle>{t("account.keys.revealed.title")}</DialogTitle>
        <DialogDescription>
          {t("account.keys.revealed.body")}
        </DialogDescription>
      </DialogHeader>
      <div className="space-y-2 rounded-md border border-dashed border-border bg-muted p-3">
        <div className="flex items-center justify-between gap-2">
          <code className="break-all text-sm">{minted?.key}</code>
          <Button size="sm" variant="outline" onClick={copy}>
            {copied ? (
              <Check className="h-3.5 w-3.5" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </Button>
        </div>
      </div>
      <DialogFooter>
        <Button onClick={() => onOpenChange(false)}>
          {t("account.keys.revealed.done")}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
