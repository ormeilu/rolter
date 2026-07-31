import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Building2, WalletCards } from "lucide-react";
import * as React from "react";

import { PageBody } from "@/components/screen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Sheet, SheetBody, SheetFooter, SheetHeader } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import {
  createBusinessUnit,
  createCustomer,
  deleteBusinessUnit,
  deleteCustomer,
  fetchBusinessUnits,
  fetchCustomers,
  updateBusinessUnit,
  updateCustomer,
  type BusinessUnitRow,
  type CustomerRow,
} from "@/lib/api";
import { useScope } from "@/lib/scope";

// the server's slug rule, mirrored so a bad value is caught before the round
// trip. slugs are the stable attribution identity, not a display name
const SLUG = /^[a-z0-9][a-z0-9-]{0,62}$/;

const UNASSIGNED = "__none__";

function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 63);
}

// a retired unit or customer keeps its history and stops being offered for new
// attribution; deleting one removes it outright
function RetiredBadge({ retiredAt }: { retiredAt: string | null }) {
  if (!retiredAt) return null;
  return <Badge tone="neutral">RETIRED</Badge>;
}

interface EditorState {
  id: string | null;
  name: string;
  slug: string;
  allowSlugChange: boolean;
  businessUnitId: string;
}

const blank = (): EditorState => ({
  id: null,
  name: "",
  slug: "",
  allowSlugChange: false,
  businessUnitId: UNASSIGNED,
});

function slugErrorFor(form: EditorState, original: string | null): string | undefined {
  const slug = form.slug.trim() || slugify(form.name);
  if (slug && !SLUG.test(slug)) {
    return "Slug must be lowercase alphanumerics and dashes, starting with a letter or digit.";
  }
  // the server refuses a silent rename because attribution already recorded
  // against the old slug does not follow it
  if (original && slug !== original && !form.allowSlugChange) {
    return "Renaming the slug breaks attribution recorded against the old one. Confirm below to allow it.";
  }
  return undefined;
}

function Editor({
  kind,
  form,
  setForm,
  original,
  units,
  onClose,
  onSubmit,
  pending,
  error,
}: {
  kind: "unit" | "customer";
  form: EditorState;
  setForm: (f: EditorState) => void;
  original: string | null;
  units: BusinessUnitRow[];
  onClose: () => void;
  onSubmit: () => void;
  pending: boolean;
  error?: string;
}) {
  const slugError = slugErrorFor(form, original);
  const noun = kind === "unit" ? "business unit" : "customer";
  const effectiveSlug = form.slug.trim() || slugify(form.name);
  return (
    <>
      <SheetHeader
        title={form.id ? `Edit ${noun}` : `New ${noun}`}
        subtitle={effectiveSlug || "—"}
        onClose={onClose}
      />
      <SheetBody>
        <Field label="Name">
          <Input
            aria-label="Name"
            value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            placeholder={kind === "unit" ? "Platform Engineering" : "Acme Corp"}
          />
        </Field>
        <Field
          label="Slug"
          hint="Derived from the name when left blank. This is the stable identity spend is attributed by."
          error={slugError}
        >
          <Input
            aria-label="Slug"
            className="font-mono text-xs"
            value={form.slug}
            onChange={(e) => setForm({ ...form, slug: e.target.value })}
            placeholder={slugify(form.name) || "acme-corp"}
          />
        </Field>
        {original && effectiveSlug !== original && (
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <input
              type="checkbox"
              aria-label="Allow slug change"
              checked={form.allowSlugChange}
              onChange={(e) =>
                setForm({ ...form, allowSlugChange: e.target.checked })
              }
            />
            Rename the slug from{" "}
            <span className="font-mono text-foreground">{original}</span> anyway
          </label>
        )}
        {kind === "customer" && (
          <Field
            label="Business unit"
            hint="Roll this customer's spend up into a business unit, or leave it unassigned."
          >
            <Select
              aria-label="Business unit"
              value={form.businessUnitId}
              onChange={(e) =>
                setForm({ ...form, businessUnitId: e.target.value })
              }
            >
              <option value={UNASSIGNED}>Unassigned</option>
              {units
                .filter((u) => !u.retired_at || u.id === form.businessUnitId)
                .map((u) => (
                  <option key={u.id} value={u.id}>
                    {u.name}
                  </option>
                ))}
            </Select>
          </Field>
        )}
        {error && <p className="text-xs text-destructive">{error}</p>}
      </SheetBody>
      <SheetFooter>
        <div className="flex justify-end gap-2 px-[22px] py-3.5">
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={!form.name.trim() || !!slugError || pending}
            onClick={onSubmit}
          >
            {form.id ? "Save" : "Create"}
          </Button>
        </div>
      </SheetFooter>
    </>
  );
}

// shared list shell: both screens are the same CRUD over an org-scoped
// collection, differing only in the business-unit assignment column
function AttributionScreen<T extends BusinessUnitRow | CustomerRow>({
  kind,
  rows,
  units,
  isLoading,
  isError,
  error,
  onCreate,
  onUpdate,
  onRetire,
  onDelete,
  mutating,
  mutationError,
  disabled,
}: {
  kind: "unit" | "customer";
  rows: T[];
  units: BusinessUnitRow[];
  isLoading: boolean;
  isError: boolean;
  error?: Error;
  onCreate: (form: EditorState) => void;
  onUpdate: (form: EditorState, original: T) => void;
  onRetire: (row: T, retired: boolean) => void;
  onDelete: (row: T) => void;
  mutating: boolean;
  mutationError?: Error;
  disabled: boolean;
}) {
  const [open, setOpen] = React.useState(false);
  const [form, setForm] = React.useState<EditorState>(blank);
  const [editing, setEditing] = React.useState<T | null>(null);

  const startCreate = () => {
    setEditing(null);
    setForm(blank());
    setOpen(true);
  };
  const startEdit = (row: T) => {
    setEditing(row);
    setForm({
      id: row.id,
      name: row.name,
      slug: row.slug,
      allowSlugChange: false,
      businessUnitId:
        "business_unit_id" in row && row.business_unit_id
          ? row.business_unit_id
          : UNASSIGNED,
    });
    setOpen(true);
  };

  const noun = kind === "unit" ? "business unit" : "customer";
  const plural = kind === "unit" ? "business units" : "customers";
  const unitName = (id: string | null) =>
    units.find((u) => u.id === id)?.name ?? null;

  if (isLoading) {
    return (
      <PageBody>
        <Skeleton className="h-9 w-[260px] rounded-md" />
        <Skeleton className="h-[180px] rounded-lg" />
      </PageBody>
    );
  }
  if (isError) {
    return (
      <p className="p-[22px] text-sm text-muted-foreground">
        Could not load {plural}: {error?.message}
      </p>
    );
  }

  const active = rows.filter((r) => !r.retired_at).length;

  return (
    <PageBody>
      <div className="flex flex-wrap items-center gap-3">
        <span className="text-sm text-muted-foreground">
          {rows.length} {plural} · {active} active
        </span>
        {mutationError && (
          <span className="text-xs text-destructive">{mutationError.message}</span>
        )}
        <Button className="ml-auto" disabled={disabled} onClick={startCreate}>
          + New {noun}
        </Button>
      </div>

      {rows.length === 0 ? (
        <EmptyState
          icon={kind === "unit" ? <Building2 /> : <WalletCards />}
          title={`No ${plural} yet`}
          description={
            kind === "unit"
              ? "Business units roll teams up so spend can be attributed to the part of the organisation that incurred it."
              : "Customers attribute usage and spend to the people you serve, so it can be billed on or reported per account."
          }
          actions={
            <Button disabled={disabled} onClick={startCreate}>
              + New {noun}
            </Button>
          }
        />
      ) : (
        <div className="grid gap-3.5 [grid-template-columns:repeat(auto-fill,minmax(300px,1fr))]">
          {rows.map((row) => {
            const assigned =
              "business_unit_id" in row ? unitName(row.business_unit_id) : null;
            return (
              <div
                key={row.id}
                className="flex flex-col gap-3 rounded-[10px] border border-[color:var(--border-default)] bg-card p-4"
                style={{ opacity: row.retired_at ? 0.6 : 1 }}
              >
                <div className="flex items-start gap-2.5">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-semibold">{row.name}</div>
                    <div className="truncate font-mono text-xs text-muted-foreground">
                      {row.slug}
                    </div>
                  </div>
                  <RetiredBadge retiredAt={row.retired_at} />
                </div>
                {kind === "customer" && (
                  <div className="text-xs text-muted-foreground">
                    {assigned ? (
                      <>
                        rolls up into{" "}
                        <span className="text-foreground">{assigned}</span>
                      </>
                    ) : (
                      "unassigned"
                    )}
                  </div>
                )}
                <div className="flex items-center gap-2 border-t border-[color:var(--border-subtle)] pt-2.5">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={disabled}
                    onClick={() => startEdit(row)}
                  >
                    Edit
                  </Button>
                  {/* retiring keeps the history a delete would strand */}
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={disabled || mutating}
                    onClick={() => onRetire(row, !row.retired_at)}
                  >
                    {row.retired_at ? "Restore" : "Retire"}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="ml-auto text-destructive"
                    disabled={disabled || mutating}
                    onClick={() => onDelete(row)}
                  >
                    Delete
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <Sheet open={open} onOpenChange={setOpen}>
        <Editor
          kind={kind}
          form={form}
          setForm={setForm}
          original={editing?.slug ?? null}
          units={units}
          onClose={() => setOpen(false)}
          pending={mutating}
          error={mutationError?.message}
          onSubmit={() => {
            if (editing) onUpdate(form, editing);
            else onCreate(form);
            setOpen(false);
          }}
        />
      </Sheet>
    </PageBody>
  );
}

// business units: roll teams up into cost-attributed units (#539, #563)
export function BusinessUnits() {
  const queryClient = useQueryClient();
  const scope = useScope();
  const orgId = scope.orgId as string | undefined;

  const units = useQuery({
    queryKey: ["business-units", orgId],
    queryFn: () => fetchBusinessUnits(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["business-units", orgId] });

  const create = useMutation({
    mutationFn: (f: EditorState) =>
      createBusinessUnit(orgId as string, {
        name: f.name.trim(),
        slug: f.slug.trim() || undefined,
      }),
    onSuccess: invalidate,
  });
  const update = useMutation({
    mutationFn: ({ form, row }: { form: EditorState; row: BusinessUnitRow }) =>
      updateBusinessUnit(row.id, {
        name: form.name.trim(),
        slug: form.slug.trim() || undefined,
        allow_slug_change: form.allowSlugChange,
      }),
    onSuccess: invalidate,
  });
  const retire = useMutation({
    mutationFn: ({ row, retired }: { row: BusinessUnitRow; retired: boolean }) =>
      updateBusinessUnit(row.id, { retired }),
    onSuccess: invalidate,
  });
  const remove = useMutation({
    mutationFn: (row: BusinessUnitRow) => deleteBusinessUnit(row.id),
    onSuccess: invalidate,
  });

  return (
    <AttributionScreen
      kind="unit"
      rows={units.data ?? []}
      units={units.data ?? []}
      isLoading={units.isLoading}
      isError={units.isError}
      error={units.error as Error | undefined}
      disabled={!orgId}
      mutating={
        create.isPending || update.isPending || retire.isPending || remove.isPending
      }
      mutationError={
        (create.error ?? update.error ?? retire.error ?? remove.error) as
          | Error
          | undefined
      }
      onCreate={(form) => create.mutate(form)}
      onUpdate={(form, row) => update.mutate({ form, row })}
      onRetire={(row, retired) => retire.mutate({ row, retired })}
      onDelete={(row) => remove.mutate(row)}
    />
  );
}

// customers: attribute usage and spend to the org's own customers (#539, #563)
export function Customers() {
  const queryClient = useQueryClient();
  const scope = useScope();
  const orgId = scope.orgId as string | undefined;

  const customers = useQuery({
    queryKey: ["customers", orgId],
    queryFn: () => fetchCustomers(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  // needed for the assignment dropdown and to name the unit on each card
  const units = useQuery({
    queryKey: ["business-units", orgId],
    queryFn: () => fetchBusinessUnits(orgId as string),
    enabled: !!orgId,
    retry: false,
  });
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["customers", orgId] });

  const create = useMutation({
    mutationFn: (f: EditorState) =>
      createCustomer(orgId as string, {
        name: f.name.trim(),
        slug: f.slug.trim() || undefined,
        business_unit_id:
          f.businessUnitId === UNASSIGNED ? null : f.businessUnitId,
      }),
    onSuccess: invalidate,
  });
  const update = useMutation({
    mutationFn: ({ form, row }: { form: EditorState; row: CustomerRow }) =>
      updateCustomer(row.id, {
        name: form.name.trim(),
        slug: form.slug.trim() || undefined,
        allow_slug_change: form.allowSlugChange,
        // null unassigns; the server treats an omitted field as unchanged, so
        // the editor always sends its current selection
        business_unit_id:
          form.businessUnitId === UNASSIGNED ? null : form.businessUnitId,
      }),
    onSuccess: invalidate,
  });
  const retire = useMutation({
    mutationFn: ({ row, retired }: { row: CustomerRow; retired: boolean }) =>
      updateCustomer(row.id, { retired }),
    onSuccess: invalidate,
  });
  const remove = useMutation({
    mutationFn: (row: CustomerRow) => deleteCustomer(row.id),
    onSuccess: invalidate,
  });

  return (
    <AttributionScreen
      kind="customer"
      rows={customers.data ?? []}
      units={units.data ?? []}
      isLoading={customers.isLoading}
      isError={customers.isError}
      error={customers.error as Error | undefined}
      disabled={!orgId}
      mutating={
        create.isPending || update.isPending || retire.isPending || remove.isPending
      }
      mutationError={
        (create.error ?? update.error ?? retire.error ?? remove.error) as
          | Error
          | undefined
      }
      onCreate={(form) => create.mutate(form)}
      onUpdate={(form, row) => update.mutate({ form, row })}
      onRetire={(row, retired) => retire.mutate({ row, retired })}
      onDelete={(row) => remove.mutate(row)}
    />
  );
}
