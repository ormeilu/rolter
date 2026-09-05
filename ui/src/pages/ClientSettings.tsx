import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, Plus, Trash2 } from "lucide-react";
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  fetchClientSettings,
  updateClientSettings,
  type ClientSettingsDto,
} from "@/lib/api";
import { useErrorState, useScreenReady } from "@/lib/ux-react";

// injected headers are edited as an ordered list rather than an object so a
// half-typed name does not collide with an existing key while it is being typed
interface HeaderPair {
  id: number;
  name: string;
  value: string;
}

interface FormState {
  publicBaseUrl: string;
  forwarded: string;
  injected: HeaderPair[];
  requestIdHeader: string;
}

let nextPairId = 0;
const pair = (name = "", value = ""): HeaderPair => ({ id: nextPairId++, name, value });

// the allowlist is a comma or newline separated list in the textbox; both are
// natural to paste, and neither is legal inside a header name
const splitList = (raw: string) =>
  raw
    .split(/[\n,]/)
    .map((h) => h.trim().toLowerCase())
    .filter((h) => h.length > 0);

const fromDto = (dto: ClientSettingsDto): FormState => ({
  publicBaseUrl: dto.public_base_url ?? "",
  forwarded: dto.forwarded_headers.join(", "),
  injected: Object.entries(dto.injected_headers).map(([name, value]) => pair(name, value)),
  requestIdHeader: dto.request_id_header,
});

// rfc 9110 token characters; the server enforces the same shape, so a typo is
// caught here rather than after a round trip
const HEADER_NAME = /^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/;

function validate(form: FormState, reserved: string[]): string | null {
  const url = form.publicBaseUrl.trim();
  if (url !== "" && !/^https?:\/\//.test(url)) {
    return "Base URL must start with http:// or https://.";
  }
  const forwarded = splitList(form.forwarded);
  for (const name of forwarded) {
    if (!HEADER_NAME.test(name)) return `'${name}' is not a valid header name.`;
    if (reserved.includes(name)) return `'${name}' is managed by the gateway.`;
  }
  if (forwarded.length > 64) return "At most 64 forwarded headers are allowed.";

  const seen = new Set<string>();
  for (const { name, value } of form.injected) {
    const lower = name.trim().toLowerCase();
    if (lower === "" && value.trim() === "") continue;
    if (!HEADER_NAME.test(lower)) return `'${name}' is not a valid header name.`;
    if (reserved.includes(lower)) return `'${lower}' is managed by the gateway.`;
    if (seen.has(lower)) return `'${lower}' is listed twice.`;
    seen.add(lower);
    if (/[\u0000-\u001f\u007f]/.test(value)) {
      return `Value for '${lower}' must not contain control characters.`;
    }
  }
  if (seen.size > 64) return "At most 64 injected headers are allowed.";

  const requestId = form.requestIdHeader.trim().toLowerCase();
  if (!HEADER_NAME.test(requestId)) return "Request ID header is not a valid header name.";
  if (reserved.includes(requestId)) return `'${requestId}' is managed by the gateway.`;
  return null;
}

// deployment-wide client settings, persisted via /api/v1/client-settings
// (superadmin only): the base URL the dashboard hands out, and what the gateway
// does with headers on the upstream leg
export default function ClientSettings() {
  const queryClient = useQueryClient();
  const settings = useQuery({
    queryKey: ["client-settings"],
    queryFn: fetchClientSettings,
    retry: false,
  });

  // UX stream (#805). the screen key comes from the enclosing UxScreenProvider;
  // `settings` is the query the user is actually waiting on for this screen
  useScreenReady(!settings.isLoading);
  useErrorState(!!settings.error, "client-settings");

  const [form, setForm] = React.useState<FormState | null>(null);
  const [saved, setSaved] = React.useState(false);
  React.useEffect(() => {
    if (settings.data && form === null) {
      setForm(fromDto(settings.data));
    }
  }, [settings.data, form]);

  const save = useMutation({
    mutationFn: (f: FormState) =>
      updateClientSettings({
        public_base_url: f.publicBaseUrl.trim() === "" ? null : f.publicBaseUrl.trim(),
        forwarded_headers: splitList(f.forwarded),
        injected_headers: Object.fromEntries(
          f.injected
            .map(({ name, value }) => [name.trim().toLowerCase(), value] as const)
            .filter(([name]) => name !== ""),
        ),
        request_id_header: f.requestIdHeader.trim().toLowerCase(),
      }),
    onSuccess: (dto) => {
      queryClient.setQueryData(["client-settings"], dto);
      setForm(fromDto(dto));
      setSaved(true);
    },
  });

  if (settings.isLoading) {
    return (
      <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-[148px] rounded-[10px]" />
        ))}
      </div>
    );
  }
  if (settings.isError) {
    return (
      <p className="p-[22px] text-sm text-muted-foreground">
        Client settings need superadmin access: {(settings.error as Error).message}
      </p>
    );
  }
  if (!form || !settings.data) return null;

  const dto = settings.data;
  const set = (patch: Partial<FormState>) => {
    setForm((f) => (f ? { ...f, ...patch } : f));
    setSaved(false);
  };
  const localError = validate(form, dto.reserved);
  // what a client would actually type; falls back to this dashboard's origin
  const effectiveBase =
    form.publicBaseUrl.trim() ||
    (typeof window === "undefined" ? "https://your-gateway.example.com" : window.location.origin);

  return (
    <div className="mx-auto flex max-w-[840px] flex-col gap-3.5 p-[22px]">
      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Base URL</span>
          <p className="mt-1 text-sm text-muted-foreground">
            The address clients point their SDK at. Set this when the gateway sits
            behind a load balancer or a custom domain, so the snippet below matches
            what a caller can actually reach. Leave it empty to use this
            dashboard&rsquo;s own origin.
          </p>
        </div>
        <div className="flex flex-col gap-1.5">
          <label htmlFor="client-public-base-url" className="text-xs font-medium text-[color:var(--text-secondary)]">
            Public base URL
          </label>
          <Input
            id="client-public-base-url"
            className="min-w-[320px] font-mono text-xs"
            aria-label="Public base URL"
            placeholder={effectiveBase}
            value={form.publicBaseUrl}
            onChange={(e) => set({ publicBaseUrl: e.target.value })}
          />
        </div>
        <Snippet base={effectiveBase} />
      </section>

      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Forwarded request headers</span>
          <p className="mt-1 text-sm text-muted-foreground">
            Inbound client headers passed through to the upstream provider,
            comma or newline separated. Everything not listed is dropped at the
            gateway.
          </p>
        </div>
        <textarea
          aria-label="Forwarded request headers"
          className="min-h-[72px] w-full rounded-md border border-[color:var(--border-default)] bg-transparent px-3 py-2 font-mono text-xs outline-none focus-visible:border-[color:var(--red-folk)]"
          placeholder="x-tenant-id, x-session-id"
          value={form.forwarded}
          onChange={(e) => set({ forwarded: e.target.value })}
        />
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span>Always propagated:</span>
          {dto.always_propagated.map((h) => (
            <Badge key={h} tone="info" className="font-mono">
              {h}
            </Badge>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span>Never forwarded:</span>
          {dto.reserved.map((h) => (
            <Badge key={h} tone="neutral" className="font-mono">
              {h}
            </Badge>
          ))}
        </div>
      </section>

      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Injected upstream headers</span>
          <p className="mt-1 text-sm text-muted-foreground">
            Static headers the gateway adds to every upstream request. A
            trace-context header of the same name still wins, so correlation
            cannot be broken from here. Values are treated as deployment secrets
            and are never written to the audit log.
          </p>
        </div>
        <div className="flex flex-col gap-2">
          {form.injected.length === 0 && (
            <p className="text-xs text-[color:var(--text-subtle)]">
              No headers injected — upstream requests carry only what the gateway
              and the caller already send.
            </p>
          )}
          {form.injected.map((row, i) => (
            <div key={row.id} className="flex items-center gap-2">
              <Input
                className="max-w-[220px] font-mono text-xs"
                aria-label={`Injected header name ${i + 1}`}
                placeholder="x-partner-id"
                value={row.name}
                onChange={(e) =>
                  set({
                    injected: form.injected.map((r) =>
                      r.id === row.id ? { ...r, name: e.target.value } : r,
                    ),
                  })
                }
              />
              <Input
                className="flex-1 font-mono text-xs"
                aria-label={`Injected header value ${i + 1}`}
                placeholder="value"
                value={row.value}
                onChange={(e) =>
                  set({
                    injected: form.injected.map((r) =>
                      r.id === row.id ? { ...r, value: e.target.value } : r,
                    ),
                  })
                }
              />
              <button
                type="button"
                aria-label={`Remove injected header ${i + 1}`}
                className="flex h-8 w-8 flex-none items-center justify-center rounded-md text-[color:var(--text-subtle)] transition-colors hover:text-[color:var(--status-danger)] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                onClick={() =>
                  set({ injected: form.injected.filter((r) => r.id !== row.id) })
                }
              >
                <Trash2 className="h-4 w-4" />
              </button>
            </div>
          ))}
        </div>
        <div>
          <Button
            variant="outline"
            onClick={() => set({ injected: [...form.injected, pair()] })}
          >
            <Plus className="mr-1.5 h-3.5 w-3.5" />
            Add header
          </Button>
        </div>
      </section>

      <section className="flex flex-col gap-3.5 rounded-[10px] border border-[color:var(--border-subtle)] p-4">
        <div>
          <span className="text-sm font-medium">Correlation</span>
          <p className="mt-1 text-sm text-muted-foreground">
            Header carrying the end-to-end request id. The gateway reuses the
            caller&rsquo;s value when present, mints one when absent, and echoes it
            on the response so a client can find its own call in the logs.
          </p>
        </div>
        <div className="flex flex-col gap-1.5">
          <label htmlFor="client-request-id-header" className="text-xs font-medium text-[color:var(--text-secondary)]">
            Request ID header
          </label>
          <Input
            id="client-request-id-header"
            className="max-w-[240px] font-mono text-xs"
            aria-label="Request ID header"
            value={form.requestIdHeader}
            onChange={(e) => set({ requestIdHeader: e.target.value })}
          />
        </div>
      </section>

      <div className="sticky bottom-0 flex items-center justify-end gap-3 border-t border-[color:var(--border-subtle)] bg-background py-3">
        {localError && <span className="text-xs text-destructive">{localError}</span>}
        {!localError && save.isError && (
          <span className="text-xs text-destructive">{(save.error as Error).message}</span>
        )}
        {saved && (
          <span className="text-xs text-[color:var(--status-success)]">
            Client settings updated.
          </span>
        )}
        <Button disabled={save.isPending || localError !== null} onClick={() => save.mutate(form)}>
          {save.isPending ? "Saving…" : "Save Changes"}
        </Button>
      </div>
    </div>
  );
}

function Snippet({ base }: { base: string }) {
  const [copied, setCopied] = React.useState(false);
  const code = `curl ${base}/v1/chat/completions \\\n  -H "Authorization: Bearer $ROLTER_KEY" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model":"fake-llm","messages":[{"role":"user","content":"ping"}]}'`;
  return (
    <div className="relative rounded-md border border-[color:var(--border-subtle)] bg-[color:var(--surface-subtle)] p-3">
      <pre className="overflow-x-auto whitespace-pre font-mono text-[0.6875rem] leading-relaxed text-[color:var(--text-secondary)]">
        {code}
      </pre>
      <button
        type="button"
        aria-label="Copy example request"
        className="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-md text-[color:var(--text-subtle)] transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        onClick={() => {
          // clipboard access is denied in some embedded contexts; the snippet
          // is selectable either way, so a failure needs no error surface
          void navigator.clipboard?.writeText(code).then(
            () => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            },
            () => {},
          );
        }}
      >
        {copied ? (
          <Check className="h-3.5 w-3.5 text-[color:var(--status-success)]" />
        ) : (
          <Copy className="h-3.5 w-3.5" />
        )}
      </button>
    </div>
  );
}
