import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BookUser, Loader2, Plus } from "lucide-react";
import * as React from "react";

import { CopyButton } from "@/components/CopyButton";
import { PageBody } from "@/components/screen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Sheet, SheetBody, SheetFooter, SheetHeader } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, type TableColumn } from "@/components/ui/table";
import {
  createScimToken,
  fetchScimTokens,
  revokeScimToken,
  ApiError,
  type CreatedScimToken,
  type ScimTokenRow,
} from "@/lib/api";
import { useScope } from "@/lib/scope";

const TOKENS_QUERY_KEY = ["scim-tokens"];

// listing, creating and revoking all require Admin on the org. a 403 is a
// permission answer, not a failure, so it gets its own calm state rather than
// an error banner
function isForbidden(error: unknown): boolean {
  return error instanceof ApiError && error.status === 403;
}

function stamp(iso: string | null): string {
  return iso ? new Date(iso).toLocaleString() : "—";
}

// SCIM provisioning tokens for /api/v1/orgs/{org}/scim-tokens (#540, #563).
// the screen manages the credentials an IdP authenticates with — the SCIM
// resource endpoints under /scim/v2 are driven by the IdP, never from here
export default function UserProvisioning() {
  const queryClient = useQueryClient();
  const scope = useScope();
  const orgId = scope.orgId;

  const tokens = useQuery({
    queryKey: [...TOKENS_QUERY_KEY, orgId],
    queryFn: () => fetchScimTokens(orgId as string),
    enabled: !!orgId,
    retry: false,
  });

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: [...TOKENS_QUERY_KEY, orgId] });

  const revoke = useMutation({
    mutationFn: (id: string) => revokeScimToken(id),
    onSuccess: invalidate,
  });

  const [issueOpen, setIssueOpen] = React.useState(false);
  const [revokeTarget, setRevokeTarget] = React.useState<ScimTokenRow | null>(null);

  const forbidden = isForbidden(tokens.error);
  // no org means no place to hang a token; a 403 means this principal may look
  // at nothing here, so both disable the mint button instead of failing later
  const canManage = !!orgId && !forbidden;

  const columns: TableColumn<ScimTokenRow & Record<string, unknown>>[] = [
    { key: "name", header: "Token" },
    {
      key: "revoked_at",
      header: "Status",
      render: (_v, row) =>
        row.revoked_at ? (
          <Badge tone="danger" title={`revoked ${stamp(row.revoked_at)}`}>
            REVOKED
          </Badge>
        ) : (
          <Badge dot tone="success">
            ACTIVE
          </Badge>
        ),
    },
    {
      key: "last_used_at",
      header: "Last sync",
      render: (_v, row) =>
        row.last_used_at ? (
          stamp(row.last_used_at)
        ) : (
          <span className="text-[color:var(--text-subtle)]">never used</span>
        ),
    },
    {
      key: "created_at",
      header: "Created",
      align: "right",
      render: (_v, row) => stamp(row.created_at),
    },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (_v, row) => (
        <Button
          variant="outline"
          size="sm"
          disabled={!!row.revoked_at || revoke.isPending}
          onClick={() => setRevokeTarget(row)}
        >
          {row.revoked_at ? "Revoked" : "Revoke"}
        </Button>
      ),
    },
  ];

  if (scope.isLoading || (tokens.isLoading && !!orgId)) {
    return (
      <PageBody>
        <Skeleton className="h-9 w-[260px] rounded-md" />
        <Skeleton className="h-[220px] rounded-lg" />
      </PageBody>
    );
  }

  const rows = tokens.data ?? [];
  const active = rows.filter((t) => !t.revoked_at).length;

  return (
    <PageBody>
      <div className="flex flex-wrap items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {rows.length} tokens · {active} active. Your identity provider presents
          one of these as a bearer token and drives{" "}
          <code className="font-mono text-xs">/scim/v2/Users</code> to create,
          update and deactivate accounts in this org.
        </span>
        <div className="ml-auto">
          <Button disabled={!canManage} onClick={() => setIssueOpen(true)}>
            <Plus className="h-4 w-4" />
            Issue token
          </Button>
        </div>
      </div>

      {forbidden && (
        <p className="text-sm text-muted-foreground">
          Provisioning tokens are visible to org admins only. Ask an admin to
          issue or revoke one for your identity provider.
        </p>
      )}
      {tokens.isError && !forbidden && (
        <p className="text-sm text-destructive">
          Failed to load provisioning tokens: {(tokens.error as Error).message}
        </p>
      )}
      {revoke.isError && (
        <p className="text-sm text-destructive">{(revoke.error as Error).message}</p>
      )}

      {!forbidden &&
        (rows.length === 0 ? (
          <EmptyState uxTarget="provisioning-list"
            icon={<BookUser />}
            title="No provisioning tokens yet"
            description="Issue a token, paste it into your identity provider's SCIM connector, and it will sync users into this org. The token is shown once, at creation."
          />
        ) : (
          <Table
            columns={columns}
            data={rows as (ScimTokenRow & Record<string, unknown>)[]}
            rowKey="id"
          />
        ))}

      {orgId && (
        <IssueTokenSheet
          open={issueOpen}
          onOpenChange={setIssueOpen}
          orgId={orgId}
          onIssued={invalidate}
        />
      )}

      <Dialog
        open={!!revokeTarget}
        onOpenChange={(open) => !open && setRevokeTarget(null)}
      >
        <DialogHeader>
          <DialogTitle>Revoke provisioning token</DialogTitle>
          <DialogDescription>
            <span className="font-mono">{revokeTarget?.name}</span> stops
            authenticating on the very next request — there is no cache to wait
            out. Accounts it already provisioned are left exactly as they are:
            nobody is deactivated or logged out, the directory simply stops
            syncing until you point the IdP at a new token.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => setRevokeTarget(null)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={revoke.isPending}
            onClick={() => {
              if (!revokeTarget) return;
              revoke.mutate(revokeTarget.id, {
                onSuccess: () => setRevokeTarget(null),
              });
            }}
          >
            {revoke.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Revoke
          </Button>
        </DialogFooter>
      </Dialog>
    </PageBody>
  );
}

// mint a token, then hand over the plaintext. the secret lives in this
// component's state only and is dropped on close — the server stores a peppered
// digest, so nothing can show it a second time
function IssueTokenSheet({
  open,
  onOpenChange,
  orgId,
  onIssued,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  orgId: string;
  onIssued: () => void;
}) {
  const [name, setName] = React.useState("");
  const [issued, setIssued] = React.useState<CreatedScimToken | null>(null);

  React.useEffect(() => {
    if (open) {
      setName("");
      setIssued(null);
    }
  }, [open]);

  const create = useMutation({
    mutationFn: () => createScimToken(orgId, { name: name.trim() }),
    onSuccess: (token) => {
      setIssued(token);
      onIssued();
    },
  });

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetHeader
        title={issued ? "Copy the token now" : "Issue provisioning token"}
        subtitle={issued ? issued.name : "scim 2.0 · bearer credential"}
        onClose={() => onOpenChange(false)}
      />
      <SheetBody>
        {issued ? (
          <>
            <div className="rounded-md border border-[color:var(--border-default)] bg-[color:var(--surface-subtle)] p-3">
              <div className="flex items-start justify-between gap-2">
                <code
                  data-testid="scim-token-secret"
                  className="min-w-0 break-all font-mono text-sm text-foreground"
                >
                  {issued.secret}
                </code>
                <CopyButton value={issued.secret} label="Copy provisioning token" />
              </div>
            </div>
            <p className="text-sm font-medium text-[color:var(--status-warning)]">
              This is the only time this token is shown. Rolter stores a hash of
              it and cannot display or recover it again — if you lose it, issue a
              new token and revoke this one.
            </p>
            <p className="text-sm text-muted-foreground">
              Paste it into your identity provider's SCIM connector as the bearer
              token, alongside the base URL{" "}
              <code className="font-mono text-xs">
                https://your-rolter-host/scim/v2
              </code>
              . The token carries the org, so no tenant id goes in the URL.
            </p>
          </>
        ) : (
          <>
            <Field
              label="Name"
              hint="how you will recognise it later — usually the identity provider it belongs to"
            >
              <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Okta production"
              />
            </Field>
            <p className="text-sm text-muted-foreground">
              Provisioned accounts join this org as viewers and have no local
              password: SCIM decides who exists, you still decide what they may
              do.
            </p>
            {create.isError && (
              <p className="text-sm text-destructive">
                {(create.error as Error).message}
              </p>
            )}
          </>
        )}
      </SheetBody>
      <SheetFooter>
        <div className="flex justify-end gap-2 px-[22px] py-3">
          {issued ? (
            <Button onClick={() => onOpenChange(false)}>Done</Button>
          ) : (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button
                disabled={!name.trim() || create.isPending}
                onClick={() => create.mutate()}
              >
                {create.isPending && (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                )}
                Issue token
              </Button>
            </>
          )}
        </div>
      </SheetFooter>
    </Sheet>
  );
}
