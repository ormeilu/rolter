// typed fetch helpers for the rolter control api (proxied at /api in dev)

export interface TargetDto {
  provider: string;
  model?: string | null;
  weight: number;
}

export interface RouteDto {
  model: string;
  strategy: string;
  targets: TargetDto[];
}

export interface VirtualKeyDto {
  key: string;
  name?: string | null;
  models: string[];
}

export interface ProviderDto {
  name: string;
  kind: string;
  api_base: string;
}

export interface GatewayConfigDto {
  providers: ProviderDto[];
  routes: RouteDto[];
  virtual_keys: VirtualKeyDto[];
}

// bearer token from a real login (see lib/auth.tsx); attached to every request
// so the session-scoped /me/* endpoints authenticate. absent in open-mode /
// email-only sessions, where the control plane needs no auth anyway.
const TOKEN_STORAGE_KEY = "rolter.session.token";

function authHeaders(): Record<string, string> {
  try {
    const token = localStorage.getItem(TOKEN_STORAGE_KEY);
    return token ? { Authorization: `Bearer ${token}` } : {};
  } catch {
    return {};
  }
}

export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { headers: authHeaders() });
  if (!res.ok) {
    throw await apiError(res);
  }
  return (await res.json()) as T;
}

/// extract the control api's `{"error": {"message": ...}}` body when present,
/// falling back to the raw status text
async function apiError(res: Response): Promise<ApiError> {
  try {
    const body = (await res.json()) as { error?: { message?: string } };
    if (body?.error?.message) {
      return new ApiError(body.error.message, res.status);
    }
  } catch {
    // not json, fall through
  }
  return new ApiError(`request failed: ${res.status}`, res.status);
}

async function sendJson<T>(
  method: "POST" | "PUT" | "PATCH" | "DELETE",
  url: string,
  body?: unknown,
): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: {
      ...authHeaders(),
      ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    throw await apiError(res);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  return (await res.json()) as T;
}

/// thrown by the analytics fetchers when this deployment cannot serve
/// analytics at all — either the control plane has no clickhouse_url
/// configured (503 from crates/rolter-control/src/analytics.rs) or the
/// endpoint does not exist on this binary (404, an older control plane).
/// Both are deployment shape, not failure, so callers render a calm empty
/// state instead of an error banner (reserved for 5xx / network failures).
export class AnalyticsUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AnalyticsUnavailableError";
  }
}

async function getAnalytics<T>(
  url: string,
  // a 404 on a collection endpoint means the route is absent from this
  // binary; on a by-id endpoint it means that one record is missing, which
  // is a genuine not-found the caller should surface as such
  { notFoundIsUnavailable = true }: { notFoundIsUnavailable?: boolean } = {},
): Promise<T> {
  const res = await fetch(url, { headers: authHeaders() });
  if (!res.ok) {
    const err = await apiError(res);
    if (res.status === 503 || (res.status === 404 && notFoundIsUnavailable)) {
      throw new AnalyticsUnavailableError(err.message);
    }
    throw err;
  }
  return (await res.json()) as T;
}

export interface AnalyticsWindow {
  since?: string;
  until?: string;
  bucket?: string;
}

function windowParams(window: AnalyticsWindow): string {
  const params = new URLSearchParams();
  if (window.since) params.set("since", window.since);
  if (window.until) params.set("until", window.until);
  if (window.bucket) params.set("bucket", window.bucket);
  const qs = params.toString();
  return qs ? `?${qs}` : "";
}

// ClickHouse JSON rows: numeric columns may come back as strings depending on
// type/format, so callers should coerce with Number(...) when rendering
export interface AnalyticsSummary {
  requests: number | string;
  tokens: number | string;
  prompt_tokens: number | string;
  completion_tokens: number | string;
  cost_usd: number | string;
  errors: number | string;
  avg_latency_ms: number | string;
}

export interface AnalyticsTimeseriesPoint {
  bucket: string;
  requests: number | string;
  tokens: number | string;
  cost_usd: number | string;
}

export interface AnalyticsByModelRow {
  model: string;
  requests: number | string;
  tokens: number | string;
  cost_usd: number | string;
  errors: number | string;
  p50_latency_ms: number | string;
  p95_latency_ms: number | string;
}

export function fetchAnalyticsSummary(
  window: AnalyticsWindow = {},
): Promise<AnalyticsSummary | undefined> {
  return getAnalytics<DataEnvelope<AnalyticsSummary>>(
    `/api/v1/analytics/summary${windowParams(window)}`,
  ).then((r) => r.data[0]);
}

export function fetchAnalyticsTimeseries(
  window: AnalyticsWindow = {},
): Promise<AnalyticsTimeseriesPoint[]> {
  return getAnalytics<DataEnvelope<AnalyticsTimeseriesPoint>>(
    `/api/v1/analytics/timeseries${windowParams(window)}`,
  ).then((r) => r.data);
}

export function fetchAnalyticsByModel(
  window: AnalyticsWindow = {},
): Promise<AnalyticsByModelRow[]> {
  return getAnalytics<DataEnvelope<AnalyticsByModelRow>>(
    `/api/v1/analytics/by-model${windowParams(window)}`,
  ).then((r) => r.data);
}

// one row of the `request_logs` table: a single gateway invocation. numeric
// columns may arrive as strings from ClickHouse JSON, so coerce when rendering.
export interface InvocationRow {
  ts: string;
  request_id: string;
  trace_id: string;
  org_id: string;
  team_id: string;
  project_id: string;
  virtual_key_id: string;
  model: string;
  provider: string;
  target: string;
  variant: string;
  status: number | string;
  stream: number | string;
  cache_hit: number | string;
  cache_read_tokens: number | string;
  cache_write_tokens: number | string;
  prompt_tokens: number | string;
  completion_tokens: number | string;
  total_tokens: number | string;
  cost_usd: number | string;
  latency_ms: number | string;
  ttft_ms: number | string;
  error: string;
  request_payload?: string;
  response_payload?: string;
}

export interface InvocationsQuery extends AnalyticsWindow {
  model?: string;
  key?: string;
  status?: "all" | "error" | "success";
  limit?: number;
  offset?: number;
}

export function fetchInvocations(
  query: InvocationsQuery = {},
): Promise<InvocationRow[]> {
  const params = new URLSearchParams();
  if (query.since) params.set("since", query.since);
  if (query.until) params.set("until", query.until);
  if (query.model) params.set("model", query.model);
  if (query.key) params.set("key", query.key);
  if (query.status) params.set("status", query.status);
  if (query.limit != null) params.set("limit", String(query.limit));
  if (query.offset != null) params.set("offset", String(query.offset));
  const qs = params.toString();
  return getAnalytics<DataEnvelope<InvocationRow>>(
    `/api/v1/analytics/invocations${qs ? `?${qs}` : ""}`,
  ).then((r) => r.data);
}

export function fetchConfig(): Promise<GatewayConfigDto> {
  return getJson<GatewayConfigDto>("/api/v1/config");
}

export function fetchRoles(): Promise<string[]> {
  return getJson<string[]>("/api/v1/roles");
}

// provider stability rollups over provider_health_events (ROL-198)

export interface UptimeRow {
  provider: string;
  target_id: string;
  events: number;
  ok: number;
  errors: number;
  timeouts: number;
  uptime: number;
  failure_rate: number;
  error_budget_burn: number;
  sla_breached: number;
  last_event: string;
}

export interface MttrRow {
  provider: string;
  target_id: string;
  mttr_seconds: number;
  incidents: number;
}

export interface TimelineRow {
  bucket: string;
  provider: string;
  target_id: string;
  events: number;
  ok: number;
  errors: number;
  timeouts: number;
}

interface DataEnvelope<T> {
  data: T[];
}

export function fetchUptime(sla = 0.99): Promise<UptimeRow[]> {
  return getJson<DataEnvelope<UptimeRow>>(
    `/api/v1/health/uptime?sla=${sla}`,
  ).then((r) => r.data);
}

export function fetchMttr(): Promise<MttrRow[]> {
  return getJson<DataEnvelope<MttrRow>>("/api/v1/health/mttr").then(
    (r) => r.data,
  );
}

export function fetchHealthTimeline(bucket = "hour"): Promise<TimelineRow[]> {
  return getJson<DataEnvelope<TimelineRow>>(
    `/api/v1/health/timeline?bucket=${bucket}`,
  ).then((r) => r.data);
}

// --- control-plane CRUD (only reachable when rolter-control is started
// with --database-url; see crates/rolter-control/src/crud.rs) ---

export const PROVIDER_KINDS = [
  "openai",
  "anthropic",
  "openai_compatible",
  "ollama",
  "ollama_cloud",
  "llama_cpp",
  "openrouter",
  "tei",
  "azure_openai",
  "bedrock",
  "vertex",
  "gemini",
  "gemini_native",
  "mistral",
  "groq",
  "xai",
  "meta_llama_api",
  "cohere",
  "perplexity",
  "together",
  "fireworks",
  "databricks",
  "aleph_alpha",
  "nebius",
  "ovhcloud",
  "scaleway",
  "deepseek",
  "qwen",
  "zhipu",
  "kimi",
  "ernie",
  "doubao",
  "hunyuan",
  "yi",
  "minimax",
  "baichuan",
  "gigachat",
  "yandex_gpt",
  "cloud_ru",
  "mts_ai",
  "naver",
  "upstage",
  "rinna",
  "rakuten",
  "sarvam",
  "krutrim",
  "falcon",
] as const;

export const STRATEGIES = [
  "round_robin",
  "random",
  "power_of_two",
  "consistent_hash",
  "cache_aware",
  "weighted",
  "pipeline",
] as const;

export interface OrgRow {
  id: string;
  name: string;
  slug: string;
  created_at: string;
}

export interface TeamRow {
  id: string;
  org_id: string;
  name: string;
  created_at: string;
}

export interface ProjectRow {
  id: string;
  team_id: string;
  name: string;
  created_at: string;
}

export function fetchOrgs(): Promise<OrgRow[]> {
  return getJson<OrgRow[]>("/api/v1/orgs");
}

export function fetchTeams(orgId: string): Promise<TeamRow[]> {
  return getJson<TeamRow[]>(`/api/v1/orgs/${orgId}/teams`);
}

export function fetchProjects(teamId: string): Promise<ProjectRow[]> {
  return getJson<ProjectRow[]>(`/api/v1/teams/${teamId}/projects`);
}

export function createOrg(input: {
  name: string;
  slug: string;
}): Promise<OrgRow> {
  return sendJson<OrgRow>("POST", "/api/v1/orgs", input);
}

export function deleteOrg(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/orgs/${id}`);
}

export function createTeam(
  orgId: string,
  input: { name: string },
): Promise<TeamRow> {
  return sendJson<TeamRow>("POST", `/api/v1/orgs/${orgId}/teams`, input);
}

export function deleteTeam(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/teams/${id}`);
}

// ---------------------------------------------------------------------------
// cost attribution: business units roll teams up, customers attribute spend to
// the org's own customers. both are retired rather than deleted once they have
// history, so `retired_at` is part of the row, not a separate lookup

export interface BusinessUnitRow {
  id: string;
  org_id: string;
  name: string;
  slug: string;
  retired_at: string | null;
  created_at: string;
}

export interface CustomerRow {
  id: string;
  org_id: string;
  business_unit_id: string | null;
  name: string;
  slug: string;
  retired_at: string | null;
  created_at: string;
}

export function fetchBusinessUnits(orgId: string): Promise<BusinessUnitRow[]> {
  return getJson<BusinessUnitRow[]>(`/api/v1/orgs/${orgId}/business-units`);
}

export function createBusinessUnit(
  orgId: string,
  input: { name: string; slug?: string },
): Promise<BusinessUnitRow> {
  return sendJson<BusinessUnitRow>(
    "POST",
    `/api/v1/orgs/${orgId}/business-units`,
    input,
  );
}

// a slug change breaks whatever already attributes spend by it, so the server
// demands allow_slug_change before it will move one
export function updateBusinessUnit(
  id: string,
  input: {
    name?: string;
    slug?: string;
    allow_slug_change?: boolean;
    retired?: boolean;
  },
): Promise<BusinessUnitRow> {
  return sendJson<BusinessUnitRow>("PUT", `/api/v1/business-units/${id}`, input);
}

export function deleteBusinessUnit(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/business-units/${id}`);
}

export function fetchCustomers(orgId: string): Promise<CustomerRow[]> {
  return getJson<CustomerRow[]>(`/api/v1/orgs/${orgId}/customers`);
}

export function createCustomer(
  orgId: string,
  input: { name: string; slug?: string; business_unit_id?: string | null },
): Promise<CustomerRow> {
  return sendJson<CustomerRow>("POST", `/api/v1/orgs/${orgId}/customers`, input);
}

export function updateCustomer(
  id: string,
  // business_unit_id follows the server's three-state contract: omit to leave
  // unchanged, null to unassign, an id to move it
  input: {
    name?: string;
    slug?: string;
    allow_slug_change?: boolean;
    business_unit_id?: string | null;
    retired?: boolean;
  },
): Promise<CustomerRow> {
  return sendJson<CustomerRow>("PUT", `/api/v1/customers/${id}`, input);
}

export function deleteCustomer(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/customers/${id}`);
}

export function createProject(
  teamId: string,
  input: { name: string },
): Promise<ProjectRow> {
  return sendJson<ProjectRow>(
    "POST",
    `/api/v1/teams/${teamId}/projects`,
    input,
  );
}

export function deleteProject(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/projects/${id}`);
}

export interface ProviderRow {
  id: string;
  org_id: string;
  name: string;
  /** stable, URL-safe identity used for `provider-slug/model` addressing */
  slug: string;
  kind: string;
  api_base: string;
  api_key_env?: string | null;
  egress_proxy?: string | null;
  created_at: string;
}

export interface CreateProviderInput {
  name: string;
  /** omit to derive a slug from the name; immutable after create */
  slug?: string;
  kind: string;
  api_base: string;
  api_key?: string;
  api_key_env?: string;
  egress_proxy?: string;
}

export interface UpdateProviderInput {
  kind?: string;
  api_base?: string;
  api_key?: string;
  api_key_env?: string;
  egress_proxy?: string;
}

export function fetchProviders(orgId: string): Promise<ProviderRow[]> {
  return getJson<ProviderRow[]>(`/api/v1/orgs/${orgId}/providers`);
}

export function createProvider(
  orgId: string,
  input: CreateProviderInput,
): Promise<ProviderRow> {
  return sendJson<ProviderRow>(
    "POST",
    `/api/v1/orgs/${orgId}/providers`,
    input,
  );
}

export function updateProvider(
  id: string,
  input: UpdateProviderInput,
): Promise<ProviderRow> {
  return sendJson<ProviderRow>("PUT", `/api/v1/providers/${id}`, input);
}

export function deleteProvider(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/providers/${id}`);
}

// --- provider groups (ADR-0022): unify a fleet of providers behind one
// `group-slug/model` address, balanced by a chosen strategy. the CRUD API
// returns default/DB groups (editable); config-owned readonly groups live only
// in the gateway snapshot and are refused by mutations with a 4xx.

export interface ProviderGroupMember {
  group_id: string;
  provider_id: string;
  provider_name: string;
  /** null = passthrough of the requested model */
  upstream_model?: string | null;
  weight: number;
  position: number;
}

export interface ProviderGroupRow {
  id: string;
  org_id: string;
  name: string;
  /** stable, URL-safe identity used for `group-slug/model` addressing */
  slug: string;
  strategy: string;
  created_at: string;
  members: ProviderGroupMember[];
}

export interface GroupMemberInput {
  provider_id: string;
  /** omit/blank for passthrough of the requested model */
  upstream_model?: string;
  weight?: number;
}

export interface CreateProviderGroupInput {
  name: string;
  /** omit to derive a slug from the name; immutable after create */
  slug?: string;
  strategy: string;
  members: GroupMemberInput[];
}

export interface UpdateProviderGroupInput {
  name?: string;
  slug?: string;
  /** required to change the otherwise-immutable slug */
  allow_slug_change?: boolean;
  strategy?: string;
  /** present = replace the whole membership; omit = leave unchanged */
  members?: GroupMemberInput[];
}

export function fetchProviderGroups(orgId: string): Promise<ProviderGroupRow[]> {
  return getJson<ProviderGroupRow[]>(`/api/v1/orgs/${orgId}/provider-groups`);
}

export function createProviderGroup(
  orgId: string,
  input: CreateProviderGroupInput,
): Promise<ProviderGroupRow> {
  return sendJson<ProviderGroupRow>(
    "POST",
    `/api/v1/orgs/${orgId}/provider-groups`,
    input,
  );
}

export function updateProviderGroup(
  id: string,
  input: UpdateProviderGroupInput,
): Promise<ProviderGroupRow> {
  return sendJson<ProviderGroupRow>("PUT", `/api/v1/provider-groups/${id}`, input);
}

export function deleteProviderGroup(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/provider-groups/${id}`);
}

export interface RouteRow {
  id: string;
  project_id: string;
  model: string;
  strategy: string;
  enabled: boolean;
  params: Record<string, unknown>;
  param_policy: Record<string, unknown>;
  created_at: string;
}

export interface RouteTargetRow {
  id: string;
  route_id: string;
  provider_id: string;
  upstream_model?: string | null;
  weight: number;
  created_at: string;
}

export function fetchRoutes(projectId: string): Promise<RouteRow[]> {
  return getJson<RouteRow[]>(`/api/v1/projects/${projectId}/routes`);
}

export function createRoute(
  projectId: string,
  input: { model: string; strategy: string },
): Promise<RouteRow> {
  return sendJson<RouteRow>(
    "POST",
    `/api/v1/projects/${projectId}/routes`,
    input,
  );
}

export function setRouteEnabled(
  id: string,
  enabled: boolean,
): Promise<RouteRow> {
  return sendJson<RouteRow>("PUT", `/api/v1/routes/${id}`, { enabled });
}

export function deleteRoute(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/routes/${id}`);
}

export function updateRouteParams(
  id: string,
  params: Record<string, unknown>,
  paramPolicy: Record<string, unknown>,
): Promise<RouteRow> {
  return sendJson<RouteRow>("PUT", `/api/v1/routes/${id}/params`, {
    params,
    param_policy: paramPolicy,
  });
}

export function fetchRouteTargets(routeId: string): Promise<RouteTargetRow[]> {
  return getJson<RouteTargetRow[]>(`/api/v1/routes/${routeId}/targets`);
}

export function createRouteTarget(
  routeId: string,
  input: { provider_id: string; upstream_model?: string; weight?: number },
): Promise<RouteTargetRow> {
  return sendJson<RouteTargetRow>(
    "POST",
    `/api/v1/routes/${routeId}/targets`,
    input,
  );
}

export function deleteRouteTarget(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/route-targets/${id}`);
}

// effective model list — bootstrap-config routes (read-only) merged with
// DB-defined routes (full CRUD), as served to the gateway
export interface EffectiveModelDto {
  model: string;
  strategy: string;
  targets: number;
  source: "config" | "db";
}

export function fetchModels(): Promise<EffectiveModelDto[]> {
  return getJson<EffectiveModelDto[]>("/api/v1/models");
}

export function deleteModel(model: string): Promise<void> {
  return sendJson<void>(
    "DELETE",
    `/api/v1/models/${encodeURIComponent(model)}`,
  );
}

// --- virtual keys (crates/rolter-control/src/crud.rs) ---

export interface VirtualKeyRow {
  id: string;
  project_id: string;
  key_hash: string;
  key_prefix: string;
  name?: string | null;
  models: string[];
  disabled: boolean;
  expires_at?: string | null;
  /// per-key response-cache override; null inherits the route decision
  cache_enabled?: boolean | null;
  created_at: string;
}

// returned only from createVirtualKey — carries the plaintext secret, shown
// once and never persisted beyond the create mutation's immediate result
export interface CreatedVirtualKey extends VirtualKeyRow {
  key: string;
}

export interface CreateVirtualKeyInput {
  name?: string;
  models?: string[];
  cache?: boolean | null;
}

export function fetchVirtualKeys(projectId: string): Promise<VirtualKeyRow[]> {
  return getJson<VirtualKeyRow[]>(`/api/v1/projects/${projectId}/virtual-keys`);
}

export function createVirtualKey(
  projectId: string,
  input: CreateVirtualKeyInput,
): Promise<CreatedVirtualKey> {
  return sendJson<CreatedVirtualKey>(
    "POST",
    `/api/v1/projects/${projectId}/virtual-keys`,
    input,
  );
}

export function setVirtualKeyDisabled(
  id: string,
  disabled: boolean,
): Promise<VirtualKeyRow> {
  return sendJson<VirtualKeyRow>("PUT", `/api/v1/virtual-keys/${id}`, {
    disabled,
  });
}

export function setVirtualKeyCache(
  id: string,
  cache: boolean | null,
): Promise<VirtualKeyRow> {
  return sendJson<VirtualKeyRow>("PUT", `/api/v1/virtual-keys/${id}/cache`, {
    cache,
  });
}

export function deleteVirtualKey(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/virtual-keys/${id}`);
}

// --- prompt template repository (crates/rolter-control/src/crud.rs) ---

export interface PromptTemplateVariable {
  name: string;
  required: boolean;
  default?: string;
}

export type PromptDecoratorRole = "system" | "assistant" | "user";
export type PromptDecoratorPosition = "prepend" | "append";

export interface PromptTemplateDecorator {
  role: PromptDecoratorRole;
  position: PromptDecoratorPosition;
  content: string;
}

export interface PromptTemplateRow {
  id: string;
  org_id: string;
  name: string;
  slug: string;
  description?: string | null;
  published_version?: number | null;
  created_at: string;
}

export interface PromptTemplateVersionRow {
  template_id: string;
  version: number;
  variables: PromptTemplateVariable[];
  decorators: PromptTemplateDecorator[];
  created_at: string;
}

export type PromptTemplateScopeType = "org" | "project" | "route" | "virtual_key";

export interface PromptTemplateScopeRow {
  template_id: string;
  version: number;
  scope_type: PromptTemplateScopeType;
  scope_id: string;
  created_at: string;
}

export interface PromptTemplateScopeInput {
  scope_type: PromptTemplateScopeType;
  scope_id: string;
}

export function fetchPromptTemplates(orgId: string): Promise<PromptTemplateRow[]> {
  return getJson<PromptTemplateRow[]>(`/api/v1/orgs/${orgId}/prompt-templates`);
}

export function createPromptTemplate(
  orgId: string,
  input: { name: string; slug?: string; description?: string },
): Promise<PromptTemplateRow> {
  return sendJson<PromptTemplateRow>(
    "POST",
    `/api/v1/orgs/${orgId}/prompt-templates`,
    input,
  );
}

export function fetchPromptTemplateVersions(
  templateId: string,
): Promise<PromptTemplateVersionRow[]> {
  return getJson<PromptTemplateVersionRow[]>(
    `/api/v1/prompt-templates/${templateId}/versions`,
  );
}

export function createPromptTemplateVersion(
  templateId: string,
  input: {
    variables: PromptTemplateVariable[];
    decorators: PromptTemplateDecorator[];
  },
): Promise<PromptTemplateVersionRow> {
  return sendJson<PromptTemplateVersionRow>(
    "POST",
    `/api/v1/prompt-templates/${templateId}/versions`,
    input,
  );
}

export function fetchPromptTemplateScopes(
  templateId: string,
  version: number,
): Promise<PromptTemplateScopeRow[]> {
  return getJson<PromptTemplateScopeRow[]>(
    `/api/v1/prompt-templates/${templateId}/versions/${version}/scopes`,
  );
}

export function setPromptTemplateScopes(
  templateId: string,
  version: number,
  scopes: PromptTemplateScopeInput[],
): Promise<PromptTemplateScopeRow[]> {
  return sendJson<PromptTemplateScopeRow[]>(
    "PUT",
    `/api/v1/prompt-templates/${templateId}/versions/${version}/scopes`,
    { scopes },
  );
}

export function publishPromptTemplateVersion(
  templateId: string,
  version: number,
): Promise<PromptTemplateRow> {
  return sendJson<PromptTemplateRow>(
    "PUT",
    `/api/v1/prompt-templates/${templateId}/publish`,
    { version },
  );
}

export function rollbackPromptTemplateVersion(
  templateId: string,
  version: number,
): Promise<PromptTemplateRow> {
  return sendJson<PromptTemplateRow>(
    "PUT",
    `/api/v1/prompt-templates/${templateId}/rollback`,
    { version },
  );
}

// --- organization skills repository (crates/rolter-control/src/crud.rs) ---

export type SkillMinimumRole = "viewer" | "member" | "admin";

export interface SkillRow {
  id: string;
  org_id: string;
  name: string;
  slug: string;
  description: string;
  retired_at?: string | null;
  published_version?: number | null;
  allowed_team_ids: string[];
  minimum_role: SkillMinimumRole;
  created_at: string;
}

export interface SkillVersionRow {
  skill_id: string;
  version: number;
  content?: string | null;
  content_ref?: string | null;
  metadata: Record<string, unknown>;
  created_at: string;
}

export interface CreateSkillInput {
  name: string;
  slug?: string;
  description?: string;
  allowed_team_ids?: string[];
  minimum_role?: SkillMinimumRole;
}

export interface UpdateSkillInput {
  name?: string;
  description?: string;
  retired?: boolean;
  allowed_team_ids?: string[];
  minimum_role?: SkillMinimumRole;
}

export function fetchSkills(orgId: string): Promise<SkillRow[]> {
  return getJson<SkillRow[]>(`/api/v1/orgs/${orgId}/skills`);
}

export function createSkill(orgId: string, input: CreateSkillInput): Promise<SkillRow> {
  return sendJson<SkillRow>("POST", `/api/v1/orgs/${orgId}/skills`, input);
}

export function updateSkill(id: string, input: UpdateSkillInput): Promise<SkillRow> {
  return sendJson<SkillRow>("PUT", `/api/v1/skills/${id}`, input);
}

export function fetchSkillVersions(id: string): Promise<SkillVersionRow[]> {
  return getJson<SkillVersionRow[]>(`/api/v1/skills/${id}/versions`);
}

export function createSkillVersion(
  id: string,
  input:
    | { content: string; content_ref?: never; metadata: Record<string, unknown> }
    | { content?: never; content_ref: string; metadata: Record<string, unknown> },
): Promise<SkillVersionRow> {
  return sendJson<SkillVersionRow>("POST", `/api/v1/skills/${id}/versions`, input);
}

export function publishSkillVersion(id: string, version: number): Promise<SkillRow> {
  return sendJson<SkillRow>("PUT", `/api/v1/skills/${id}/publish`, { version });
}

export function rollbackSkillVersion(id: string, version: number): Promise<SkillRow> {
  return sendJson<SkillRow>("PUT", `/api/v1/skills/${id}/rollback`, { version });
}

// --- budgets, rate limits, model pricing (crates/rolter-control/src/crud.rs) ---

export const SCOPE_TYPES = ["org", "team", "project", "virtual_key"] as const;

export interface BudgetRow {
  id: string;
  scope_type: string;
  scope_id: string;
  /// decimal, returned as text
  limit_usd: string;
  period: string;
  created_at: string;
}

export interface CreateBudgetInput {
  scope_type: string;
  scope_id: string;
  limit_usd: string;
  period?: string;
}

export function fetchBudgets(
  scopeType: string,
  scopeId: string,
): Promise<BudgetRow[]> {
  return getJson<BudgetRow[]>(
    `/api/v1/budgets?scope_type=${encodeURIComponent(scopeType)}&scope_id=${encodeURIComponent(scopeId)}`,
  );
}

export function createBudget(input: CreateBudgetInput): Promise<BudgetRow> {
  return sendJson<BudgetRow>("POST", "/api/v1/budgets", input);
}

export function deleteBudget(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/budgets/${id}`);
}

export interface RateLimitRow {
  id: string;
  scope_type: string;
  scope_id: string;
  rpm?: number | null;
  tpm?: number | null;
  created_at: string;
}

export interface CreateRateLimitInput {
  scope_type: string;
  scope_id: string;
  rpm?: number;
  tpm?: number;
}

export function fetchRateLimits(
  scopeType: string,
  scopeId: string,
): Promise<RateLimitRow[]> {
  return getJson<RateLimitRow[]>(
    `/api/v1/rate-limits?scope_type=${encodeURIComponent(scopeType)}&scope_id=${encodeURIComponent(scopeId)}`,
  );
}

export function createRateLimit(
  input: CreateRateLimitInput,
): Promise<RateLimitRow> {
  return sendJson<RateLimitRow>("POST", "/api/v1/rate-limits", input);
}

export function deleteRateLimit(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/rate-limits/${id}`);
}

export interface ModelPriceRow {
  id: string;
  model: string;
  /// decimal, returned as text
  input_per_mtok: string;
  output_per_mtok: string;
  cached_input_per_mtok?: string | null;
  currency: string;
  created_at: string;
}

export interface UpsertModelPriceInput {
  model: string;
  input_per_mtok: string;
  output_per_mtok: string;
  cached_input_per_mtok?: string;
  currency?: string;
}

export function fetchModelPrices(): Promise<ModelPriceRow[]> {
  return getJson<ModelPriceRow[]>("/api/v1/model-prices");
}

export function upsertModelPrice(
  input: UpsertModelPriceInput,
): Promise<ModelPriceRow> {
  return sendJson<ModelPriceRow>("PUT", "/api/v1/model-prices", input);
}

export function deleteModelPrice(model: string): Promise<void> {
  return sendJson<void>(
    "DELETE",
    `/api/v1/model-prices/${encodeURIComponent(model)}`,
  );
}

// --- local-account auth (crates/rolter-control/src/auth.rs, ROL-32) ---

export interface LoginResponse {
  /** opaque bearer token; store it and send as Authorization: Bearer */
  token: string;
  expires_at: string;
  user: { id: string; email: string; is_superadmin: boolean };
}

// authenticate a local account; returns a session token. rejects (throws) on
// bad credentials or when local accounts aren't configured.
export function login(email: string, password: string): Promise<LoginResponse> {
  return sendJson<LoginResponse>("POST", "/api/v1/auth/login", {
    email,
    password,
  });
}

/** what the login screen may offer; see crates/rolter-control/src/auth_policy.rs */
export interface AuthMethods {
  /** render the email + password form */
  password: boolean;
  /** one entry per enabled identity provider; empty when no IdP is configured */
  sso: { slug: string; name: string; start_url: string }[];
}

// unauthenticated by necessity: read before anyone has a session, so the login
// screen knows whether this deployment uses passwords, sso, or both
export function getAuthMethods(): Promise<AuthMethods> {
  return getJson<AuthMethods>("/api/v1/auth/methods");
}

export function logout(): Promise<void> {
  return sendJson<void>("POST", "/api/v1/auth/logout");
}

// --- invitations (crates/rolter-control/src/invitations.rs, #712) ---

export interface Invitation {
  id: string;
  org_id: string;
  email: string;
  role: Role;
  team_id: string | null;
  project_id: string | null;
  invited_by: string | null;
  expires_at: string;
  accepted_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export interface CreatedInvitation {
  invitation: Invitation;
  /** shown once; the server keeps only its digest */
  token: string;
  accept_url: string;
}

export function listInvitations(orgId: string): Promise<Invitation[]> {
  return getJson<Invitation[]>(`/api/v1/orgs/${orgId}/invitations`);
}

export function createInvitation(
  orgId: string,
  body: {
    email: string;
    role: Role;
    scope_type?: "org" | "team" | "project";
    scope_id?: string;
  },
): Promise<CreatedInvitation> {
  return sendJson<CreatedInvitation>(
    "POST",
    `/api/v1/orgs/${orgId}/invitations`,
    body,
  );
}

export function revokeInvitation(id: string): Promise<Invitation> {
  return sendJson<Invitation>("DELETE", `/api/v1/invitations/${id}`);
}

export interface InvitationPreview {
  org_name: string;
  email: string;
  role: Role;
  expires_at: string;
}

// unauthenticated: the invitee has no account yet, the token is the credential
export function previewInvitation(token: string): Promise<InvitationPreview> {
  return getJson<InvitationPreview>(
    `/api/v1/invitations/accept/${encodeURIComponent(token)}`,
  );
}

export function acceptInvitation(
  token: string,
  password: string,
): Promise<LoginResponse> {
  return sendJson<LoginResponse>(
    "POST",
    `/api/v1/invitations/accept/${encodeURIComponent(token)}/accept`,
    { password },
  );
}

// --- users + memberships (crates/rolter-control/src/crud.rs, ROL-223) ---

export const ROLES = ["admin", "member", "viewer"] as const;
export type Role = (typeof ROLES)[number];

// scope types a membership can be granted at (virtual_key is not a role scope)
export const MEMBERSHIP_SCOPE_TYPES = ["org", "team", "project"] as const;
export type MembershipScopeType = (typeof MEMBERSHIP_SCOPE_TYPES)[number];

export interface UserRow {
  id: string;
  email: string;
  is_superadmin: boolean;
  /** set when the account is deactivated (login blocked); null when active */
  deactivated_at?: string | null;
  created_at: string;
}

export interface MembershipRow {
  id: string;
  user_id: string;
  org_id?: string | null;
  team_id?: string | null;
  project_id?: string | null;
  role: string;
  created_at: string;
}

// returned by inviteUser: the new account plus its initial org membership
export interface CreatedUser {
  user: UserRow;
  membership: MembershipRow;
}

export interface InviteUserInput {
  email: string;
  /** optional initial password; omit for an sso-only shell account */
  password?: string;
  /** role granted at the org; defaults to member */
  role?: string;
}

export interface UpdateUserInput {
  email?: string;
  password?: string;
  is_superadmin?: boolean;
  deactivated?: boolean;
}

export interface CreateMembershipInput {
  user_id: string;
  scope_type: MembershipScopeType;
  scope_id: string;
  role: string;
}

// every account with a membership anywhere in the org's tree
export function fetchUsers(orgId: string): Promise<UserRow[]> {
  return getJson<UserRow[]>(`/api/v1/orgs/${orgId}/users`);
}

// create/invite an account and grant it a role in the org atomically
export function inviteUser(
  orgId: string,
  input: InviteUserInput,
): Promise<CreatedUser> {
  return sendJson<CreatedUser>("POST", `/api/v1/orgs/${orgId}/users`, input);
}

export function updateUser(
  id: string,
  input: UpdateUserInput,
): Promise<UserRow> {
  return sendJson<UserRow>("PUT", `/api/v1/users/${id}`, input);
}

export function deleteUser(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/users/${id}`);
}

// every role grant scoped within the org (org/team/project)
export function fetchMemberships(orgId: string): Promise<MembershipRow[]> {
  return getJson<MembershipRow[]>(`/api/v1/orgs/${orgId}/memberships`);
}

export function createMembership(
  orgId: string,
  input: CreateMembershipInput,
): Promise<MembershipRow> {
  return sendJson<MembershipRow>(
    "POST",
    `/api/v1/orgs/${orgId}/memberships`,
    input,
  );
}

export function deleteMembership(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/memberships/${id}`);
}

// --- scim provisioning tokens (crates/rolter-control/src/scim.rs, #540) ---
//
// the dashboard only manages the tokens an IdP authenticates with; the SCIM
// resource endpoints (/scim/v2/Users) are driven by the IdP itself and are
// deliberately not called from here. every one of these requires Admin on the
// org (rolter_auth::Role::Admin).

// an issued provisioning token as the server stores it. `token_hash` is never
// serialised, so there is no field carrying the secret — a listed token can be
// identified and revoked, never read back.
export interface ScimTokenRow {
  id: string;
  org_id: string;
  name: string;
  created_by: string | null;
  created_at: string;
  /** last time an IdP presented this token; null until it is first used */
  last_used_at: string | null;
  /** set once revoked; a revoked token never authenticates again */
  revoked_at: string | null;
}

// the create response: the stored row plus the one and only sighting of the
// plaintext bearer value
export interface CreatedScimToken extends ScimTokenRow {
  /** the bearer value the IdP is configured with; not stored, not recoverable */
  secret: string;
}

export function fetchScimTokens(orgId: string): Promise<ScimTokenRow[]> {
  return getJson<ScimTokenRow[]>(`/api/v1/orgs/${orgId}/scim-tokens`);
}

export function createScimToken(
  orgId: string,
  input: { name: string },
): Promise<CreatedScimToken> {
  return sendJson<CreatedScimToken>(
    "POST",
    `/api/v1/orgs/${orgId}/scim-tokens`,
    input,
  );
}

// revocation returns the updated row rather than 204 — the screen uses it to
// show the revoked timestamp without a refetch race
export function revokeScimToken(id: string): Promise<ScimTokenRow> {
  return sendJson<ScimTokenRow>("DELETE", `/api/v1/scim-tokens/${id}`);
}

// --- self-service (crates/rolter-control/src/me.rs, ROL-224) ---
//
// end-user surface: manage your own virtual keys and see your own usage. these
// require a real login session (not the admin token path).

// a key the current user owns, enriched with its project/org names; never
// carries the key hash
export interface OwnedKeyRow {
  id: string;
  project_id: string;
  project_name: string;
  org_name: string;
  key_prefix: string;
  name?: string | null;
  models: string[];
  disabled: boolean;
  expires_at?: string | null;
  created_at: string;
}

// returned from mint/rotate — carries the plaintext secret, shown once
export interface MintedKey extends VirtualKeyRow {
  key: string;
}

export interface MintKeyInput {
  name?: string;
  models?: string[];
  cache?: boolean | null;
}

export interface MyUsageRow {
  virtual_key_id: string;
  requests: number | string;
  tokens: number | string;
  cost_usd: number | string;
  errors: number | string;
}

export function fetchMyKeys(): Promise<OwnedKeyRow[]> {
  return getJson<OwnedKeyRow[]>("/api/v1/me/virtual-keys");
}

export function mintMyKey(
  projectId: string,
  input: MintKeyInput,
): Promise<MintedKey> {
  return sendJson<MintedKey>(
    "POST",
    `/api/v1/me/projects/${projectId}/virtual-keys`,
    input,
  );
}

export function rotateMyKey(id: string): Promise<MintedKey> {
  return sendJson<MintedKey>("POST", `/api/v1/me/virtual-keys/${id}/rotate`);
}

export function deleteMyKey(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/me/virtual-keys/${id}`);
}

// per-key usage/spend over the window; throws AnalyticsUnavailableError (503)
// when the deployment has no ClickHouse configured
export function fetchMyUsage(
  window: AnalyticsWindow = {},
): Promise<MyUsageRow[]> {
  return getAnalytics<DataEnvelope<MyUsageRow>>(
    `/api/v1/me/usage${windowParams(window)}`,
  ).then((r) => r.data);
}

export interface AuditLogEntry {
  id: string;
  org_id?: string | null;
  actor_user_id?: string | null;
  action: string;
  target_type?: string | null;
  target_id?: string | null;
  detail?: unknown;
  at: string;
}

export interface AuditLogPage {
  items: AuditLogEntry[];
  next_cursor: string | null;
  previous_cursor: string | null;
  has_next: boolean;
  has_previous: boolean;
  total?: number;
}

export interface AuditLogQuery {
  limit?: number;
  cursor?: string;
  direction?: "next" | "previous";
  actor?: string;
  action?: string;
  target_type?: string;
  from?: string;
  to?: string;
  include_total?: boolean;
}

export function fetchAuditLogPage(
  orgId: string,
  query: AuditLogQuery = {},
): Promise<AuditLogPage> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== "") {
      params.set(key, String(value));
    }
  }
  const qs = params.toString();
  return getJson<AuditLogPage>(
    `/api/v1/orgs/${orgId}/audit-log${qs ? `?${qs}` : ""}`,
  );
}

// ---------------------------------------------------------------------------
// security settings (superadmin-only global gateway policy)

export interface SecuritySettingsDto {
  virtual_key_required: boolean;
  allow_direct_provider_keys: boolean;
  allowed_origins: string[];
  allowed_headers: string[];
  required_headers: Record<string, string>;
  auth_bypass_routes: string[];
  dashboard_auth_enabled: boolean;
  dashboard_credential_ref: string | null;
  dashboard_secret_configured: boolean;
  updated_at: string;
}

export interface UpdateSecuritySettingsInput {
  virtual_key_required: boolean;
  allow_direct_provider_keys: boolean;
  allowed_origins: string[];
  allowed_headers: string[];
  required_headers: Record<string, string>;
  auth_bypass_routes: string[];
  dashboard_auth_enabled: boolean;
  dashboard_credential_ref?: string | null;
  /// write-only; sealed server-side, never echoed back
  managed_dashboard_secret?: string;
}

export function fetchSecuritySettings(): Promise<SecuritySettingsDto> {
  return getJson<SecuritySettingsDto>("/api/v1/security-settings");
}

export function updateSecuritySettings(
  input: UpdateSecuritySettingsInput,
): Promise<SecuritySettingsDto> {
  return sendJson<SecuritySettingsDto>("PUT", "/api/v1/security-settings", input);
}

// ---------------------------------------------------------------------------
// compatibility policy: values applied when translating between the OpenAI and
// Anthropic wire formats

export interface CompatibilityPolicyDto {
  anthropic_version: string;
  default_max_tokens: number;
  updated_at: string;
  // fields that only take effect after a gateway restart; empty today, but the
  // server owns the list so the screen can warn without knowing which they are
  restart_required: string[];
}

export interface UpdateCompatibilityPolicyInput {
  anthropic_version: string;
  default_max_tokens: number;
}

export function fetchCompatibilityPolicy(): Promise<CompatibilityPolicyDto> {
  return getJson<CompatibilityPolicyDto>("/api/v1/compatibility-policy");
}

export function updateCompatibilityPolicy(
  input: UpdateCompatibilityPolicyInput,
): Promise<CompatibilityPolicyDto> {
  return sendJson<CompatibilityPolicyDto>(
    "PUT",
    "/api/v1/compatibility-policy",
    input,
  );
}

// ---------------------------------------------------------------------------
// runtime policy: retry, timeout and admission-queue controls

export const BACKPRESSURE_POLICIES = ["drop", "block", "error"] as const;

export type BackpressurePolicy = (typeof BACKPRESSURE_POLICIES)[number];

export interface RuntimePolicyDto {
  retry_max_retries: number;
  retry_base_ms: number;
  retry_max_ms: number;
  timeout_connect_s: number;
  timeout_request_s: number;
  queue_enabled: boolean;
  queue_capacity: number;
  queue_workers: number;
  queue_backpressure: BackpressurePolicy;
  queue_block_ms: number;
  updated_at: string;
}

export type UpdateRuntimePolicyInput = Omit<RuntimePolicyDto, "updated_at">;

export function fetchRuntimePolicy(): Promise<RuntimePolicyDto> {
  return getJson<RuntimePolicyDto>("/api/v1/runtime-policy");
}

export function updateRuntimePolicy(
  input: UpdateRuntimePolicyInput,
): Promise<RuntimePolicyDto> {
  return sendJson<RuntimePolicyDto>("PUT", "/api/v1/runtime-policy", input);
}

// ---------------------------------------------------------------------------
// client settings: advertised base URL and upstream header handling (#564)

export interface ClientSettingsDto {
  public_base_url: string | null;
  forwarded_headers: string[];
  injected_headers: Record<string, string>;
  request_id_header: string;
  updated_at: string;
  /// header names propagated whether or not they are listed
  always_propagated: string[];
  /// header names that can never be forwarded or injected
  reserved: string[];
}

export type UpdateClientSettingsInput = Pick<
  ClientSettingsDto,
  "public_base_url" | "forwarded_headers" | "injected_headers" | "request_id_header"
>;

export function fetchClientSettings(): Promise<ClientSettingsDto> {
  return getJson<ClientSettingsDto>("/api/v1/client-settings");
}

export function updateClientSettings(
  input: UpdateClientSettingsInput,
): Promise<ClientSettingsDto> {
  return sendJson<ClientSettingsDto>("PUT", "/api/v1/client-settings", input);
}

// ---------------------------------------------------------------------------
// model defaults: inference parameters filled in when a client omits them (#564)

export interface ModelDefaultsDto {
  enabled: boolean;
  default_model: string | null;
  default_temperature: number | null;
  default_top_p: number | null;
  default_max_tokens: number | null;
  updated_at: string;
}

export type UpdateModelDefaultsInput = Omit<ModelDefaultsDto, "updated_at">;

export function fetchModelDefaults(): Promise<ModelDefaultsDto> {
  return getJson<ModelDefaultsDto>("/api/v1/model-defaults");
}

export function updateModelDefaults(
  input: UpdateModelDefaultsInput,
): Promise<ModelDefaultsDto> {
  return sendJson<ModelDefaultsDto>("PUT", "/api/v1/model-defaults", input);
}

// ---------------------------------------------------------------------------
// logging settings: request-log sampling, payload capture and retention

export interface LoggingSettingsDto {
  sample_rate: number;
  payload_capture_enabled: boolean;
  payload_capture_max_bytes: number;
  payload_capture_redact_fields: string[];
  payload_capture_models: string[];
  payload_capture_virtual_key_ids: string[];
  retention_days: number;
  payload_retention_hours: number;
  updated_at: string;
}

export type UpdateLoggingSettingsInput = Omit<LoggingSettingsDto, "updated_at">;

export function fetchLoggingSettings(): Promise<LoggingSettingsDto> {
  return getJson<LoggingSettingsDto>("/api/v1/logging-settings");
}

export function updateLoggingSettings(
  input: UpdateLoggingSettingsInput,
): Promise<LoggingSettingsDto> {
  return sendJson<LoggingSettingsDto>("PUT", "/api/v1/logging-settings", input);
}

// ---------------------------------------------------------------------------
// feature flags: global switches for hot-reloadable gateway subsystems

// a flag whose subsystem this deployment cannot run. the screen renders these
// as unavailable rather than as a switch that silently does nothing (#535)
export interface UnavailableFlagDto {
  flag: FeatureFlagKey;
  reason: string;
}

export const FEATURE_FLAG_KEYS = [
  "response_cache",
  "cache_aware_routing",
  "circuit_breaker",
  "active_health_checks",
  "complexity_routing",
  "guardrails",
] as const;

export type FeatureFlagKey = (typeof FEATURE_FLAG_KEYS)[number];

export type FeatureFlagValues = Record<FeatureFlagKey, boolean>;

export type FeatureFlagsDto = FeatureFlagValues & {
  updated_at: string;
  unavailable: UnavailableFlagDto[];
};

export function fetchFeatureFlags(): Promise<FeatureFlagsDto> {
  return getJson<FeatureFlagsDto>("/api/v1/feature-flags");
}

export function updateFeatureFlags(
  input: FeatureFlagValues,
): Promise<FeatureFlagsDto> {
  return sendJson<FeatureFlagsDto>("PUT", "/api/v1/feature-flags", input);
}

// ---------------------------------------------------------------------------
// cluster inventory: nodes register themselves by identifying on their snapshot
// poll, so this is a read + drain/forget surface, never an enrolment one

export interface ClusterNodeRow {
  id: string;
  role: string;
  build_version: string;
  config_version: number;
  desired_state: string;
  state_changed_at: string;
  first_seen_at: string;
  last_seen_at: string;
  // polled inside the liveness window
  live: boolean;
  // reported at least the control plane's current config version
  converged: boolean;
}

export function fetchClusterNodes(): Promise<ClusterNodeRow[]> {
  return getJson<ClusterNodeRow[]>("/api/v1/cluster/nodes");
}

export function setClusterNodeDrain(
  id: string,
  draining: boolean,
): Promise<ClusterNodeRow> {
  return sendJson<ClusterNodeRow>(
    "PUT",
    `/api/v1/cluster/nodes/${encodeURIComponent(id)}/drain`,
    { draining },
  );
}

export function forgetClusterNode(id: string): Promise<void> {
  return sendJson<void>(
    "DELETE",
    `/api/v1/cluster/nodes/${encodeURIComponent(id)}`,
  );
}

// ---------------------------------------------------------------------------
// adaptive routing policy: the global kill switch and blend weights every route
// on the `adaptive` strategy is governed by

// the gateway clamps the exploration ratio to this; the server rejects anything
// above it rather than silently reinterpreting the value
export const MAX_EXPLORATION_RATIO = 0.5;
export const MAX_ADAPTIVE_WEIGHT = 100;
export const MAX_ADAPTIVE_MIN_SAMPLES = 1_000_000;

export interface AdaptiveRoutingPolicyDto {
  enabled: boolean;
  latency_weight: number;
  cost_weight: number;
  load_weight: number;
  exploration_ratio: number;
  min_samples: number;
  updated_at: string;
  // public model names of enabled routes on the `adaptive` strategy, so the
  // blast radius of the kill switch is visible before it is flipped
  affected_routes: string[];
}

export interface UpdateAdaptiveRoutingPolicyInput {
  enabled: boolean;
  latency_weight: number;
  cost_weight: number;
  load_weight: number;
  exploration_ratio: number;
  min_samples: number;
}

export function fetchAdaptiveRoutingPolicy(): Promise<AdaptiveRoutingPolicyDto> {
  return getJson<AdaptiveRoutingPolicyDto>("/api/v1/adaptive-routing-policy");
}

export function updateAdaptiveRoutingPolicy(
  input: UpdateAdaptiveRoutingPolicyInput,
): Promise<AdaptiveRoutingPolicyDto> {
  return sendJson<AdaptiveRoutingPolicyDto>(
    "PUT",
    "/api/v1/adaptive-routing-policy",
    input,
  );
}

export interface AdaptiveDecisionCountsDto {
  blend: number;
  exploration: number;
  fallback: number;
}

export interface AdaptiveTargetTelemetryDto {
  target: number;
  provider?: string;
  upstream_model?: string;
  score?: number;
  latency_score?: number;
  cost_score?: number;
  load_score?: number;
  latency_ms?: number;
  cost_per_mtok?: number;
  in_flight?: number;
  samples?: number;
  last_sample_age_ms?: number | null;
}

export interface AdaptiveNodeTelemetryDto {
  node_id: string;
  engaged: boolean;
  observed: number;
  decisions: AdaptiveDecisionCountsDto;
  policy: {
    enabled?: boolean;
    latency_weight?: number;
    cost_weight?: number;
    load_weight?: number;
    exploration_ratio?: number;
    min_samples?: number;
  };
  targets: AdaptiveTargetTelemetryDto[];
  reported_at: string;
}

export interface AdaptiveRouteTelemetryDto {
  model: string;
  engaged: boolean;
  nodes: AdaptiveNodeTelemetryDto[];
}

export interface AdaptiveRoutingTelemetryDto {
  generated_at: string;
  fresh_window_secs: number;
  routes: AdaptiveRouteTelemetryDto[];
}

export function fetchAdaptiveRoutingTelemetry(): Promise<AdaptiveRoutingTelemetryDto> {
  return getJson<AdaptiveRoutingTelemetryDto>(
    "/api/v1/adaptive-routing-telemetry",
  );
}

// ---------------------------------------------------------------------------
// alerting: channels, rules, notification history

export const ALERT_SIGNALS = [
  "error_rate",
  "p95_latency_ms",
  "spend_velocity",
  "request_volume",
  "provider_health_flaps",
] as const;

export interface AlertChannelRow {
  id: string;
  name: string;
  kind: string;
  endpoint: string;
  enabled: boolean;
  secret_configured: boolean;
  created_at: string;
  updated_at: string;
}

export interface AlertChannelInput {
  name: string;
  endpoint: string;
  enabled: boolean;
  managed_secret?: string;
}

export interface AlertRuleRow {
  id: string;
  name: string;
  signal: string;
  threshold: number;
  window_secs: number;
  channel_id: string | null;
  enabled: boolean;
  state: string;
  last_value: number | null;
  last_evaluated_at: string | null;
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface AlertRuleInput {
  name: string;
  signal: string;
  threshold: number;
  window_secs: number;
  channel_id?: string | null;
  enabled: boolean;
}

export interface AlertNotificationRow {
  id: string;
  rule_id: string;
  channel_id: string | null;
  state: string;
  delivery_status: string;
  detail: string | null;
  sent_at: string;
}

export function fetchAlertChannels(): Promise<AlertChannelRow[]> {
  return getJson<AlertChannelRow[]>("/api/v1/alert-channels");
}

export function createAlertChannel(
  input: AlertChannelInput,
): Promise<AlertChannelRow> {
  return sendJson<AlertChannelRow>("POST", "/api/v1/alert-channels", input);
}

export function updateAlertChannel(
  id: string,
  input: AlertChannelInput,
): Promise<AlertChannelRow> {
  return sendJson<AlertChannelRow>("PUT", `/api/v1/alert-channels/${id}`, input);
}

export function deleteAlertChannel(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/alert-channels/${id}`);
}

export function fetchAlertRules(): Promise<AlertRuleRow[]> {
  return getJson<AlertRuleRow[]>("/api/v1/alert-rules");
}

export function createAlertRule(input: AlertRuleInput): Promise<AlertRuleRow> {
  return sendJson<AlertRuleRow>("POST", "/api/v1/alert-rules", input);
}

export function updateAlertRule(
  id: string,
  input: AlertRuleInput,
): Promise<AlertRuleRow> {
  return sendJson<AlertRuleRow>("PUT", `/api/v1/alert-rules/${id}`, input);
}

export function deleteAlertRule(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/alert-rules/${id}`);
}

export function evaluateAlertRule(
  id: string,
): Promise<{ rule: AlertRuleRow; notified: boolean }> {
  return sendJson<{ rule: AlertRuleRow; notified: boolean }>(
    "POST",
    `/api/v1/alert-rules/${id}/evaluate`,
  );
}

export function fetchAlertHistory(
  limit = 100,
  ruleId?: string,
): Promise<AlertNotificationRow[]> {
  const params = new URLSearchParams({ limit: String(limit) });
  if (ruleId) params.set("rule_id", ruleId);
  return getJson<AlertNotificationRow[]>(
    `/api/v1/alert-notifications?${params}`,
  );
}

// ---------------------------------------------------------------------------
// observability connectors (OTLP log shipping)

export interface ConnectorRow {
  id: string;
  name: string;
  kind: string;
  endpoint: string;
  enabled: boolean;
  sampling_rate: number;
  auth_secret_ref: string | null;
  auth_secret_configured: boolean;
  health_status: string;
  health_checked_at: string | null;
  health_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface ConnectorInput {
  name: string;
  kind: "otlp_http";
  endpoint: string;
  enabled: boolean;
  sampling_rate: number;
  auth_secret_ref?: string | null;
  /// write-only bearer token, sealed before persistence
  managed_auth_secret?: string;
}

export function fetchConnectors(): Promise<ConnectorRow[]> {
  return getJson<ConnectorRow[]>("/api/v1/connectors");
}

export function createConnector(input: ConnectorInput): Promise<ConnectorRow> {
  return sendJson<ConnectorRow>("POST", "/api/v1/connectors", input);
}

export function updateConnector(
  id: string,
  input: ConnectorInput,
): Promise<ConnectorRow> {
  return sendJson<ConnectorRow>("PUT", `/api/v1/connectors/${id}`, input);
}

export function deleteConnector(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/connectors/${id}`);
}

export function testConnector(id: string): Promise<{
  delivered: boolean;
  health_status: string;
  health_checked_at: string;
}> {
  return sendJson<{
    delivered: boolean;
    health_status: string;
    health_checked_at: string;
  }>("POST", `/api/v1/connectors/${id}/test`);
}

// ---------------------------------------------------------------------------
// mcp tool-call logs (clickhouse-backed; 503 → AnalyticsUnavailableError)

export const MCP_TRANSPORTS = ["stdio", "streamable_http", "sse"] as const;
export const MCP_STATUSES = [
  "success",
  "error",
  "timeout",
  "auth_denied",
  "transport_error",
] as const;

export interface McpLogRow {
  ts: string;
  event_id: string;
  server: string;
  tool: string;
  transport: string;
  status: string;
  latency_ms: number;
  org_id: string;
  team_id: string;
  project_id: string;
  virtual_key_id: string;
  user_id: string;
  request_id: string;
  trace_id: string;
  error: string | null;
}

export interface McpLogDetail extends McpLogRow {
  arguments: string | null;
  result: string | null;
}

export interface McpLogsQuery extends AnalyticsWindow {
  server?: string;
  tool?: string;
  transport?: string;
  status?: string;
  key?: string;
  user?: string;
  limit?: number;
  cursor?: string;
}

export interface McpSummaryRow {
  calls: string | number;
  failures: string | number;
  avg_latency_ms: number;
  p95_latency_ms: number;
}

export function fetchMcpLogs(
  query: McpLogsQuery = {},
): Promise<{ data: McpLogRow[]; next_cursor: string | null }> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== "") {
      params.set(key, String(value));
    }
  }
  const qs = params.toString();
  return getAnalytics<{ data: McpLogRow[]; next_cursor: string | null }>(
    `/api/v1/mcp/logs${qs ? `?${qs}` : ""}`,
  );
}

export function fetchMcpSummary(
  window: AnalyticsWindow = {},
): Promise<McpSummaryRow | undefined> {
  return getAnalytics<{ data: McpSummaryRow[] }>(
    `/api/v1/mcp/logs/summary${windowParams(window)}`,
  ).then((r) => r.data[0]);
}

export function fetchMcpLogDetail(eventId: string): Promise<McpLogDetail> {
  // by-id: a 404 here is "no such event", not "analytics is unavailable"
  return getAnalytics<McpLogDetail>(
    `/api/v1/mcp/logs/${encodeURIComponent(eventId)}`,
    { notFoundIsUnavailable: false },
  );
}

// ---------------------------------------------------------------------------
// mcp servers, oauth consent grants and token sessions
// (crates/rolter-control/src/mcp_oauth.rs, #561)
//
// three properties of that module are load-bearing for the screens below:
// no token material ever crosses this boundary (a session carries metadata and
// `has_refresh_token`, never a token); a grant is the unit of consent, so
// revoking one revokes its sessions in the same transaction; and a listing is
// owner-scoped — an org admin sees the whole org, everyone else sees only the
// rows they own

export interface McpServerRow {
  id: string;
  org_id: string;
  name: string;
  slug: string;
  url: string;
  transport: string;
  description: string;
  enabled: boolean;
  tools: string[];
  source: "custom" | "library";
  required_scopes: string[];
  created_at: string;
}

export interface McpServerInput {
  name: string;
  slug: string;
  url: string;
  transport: string;
  description: string;
  enabled: boolean;
  tools: string[];
  source: "custom" | "library";
  required_scopes: string[];
}

export interface McpLibraryItem {
  slug: string;
  name: string;
  description: string;
  url: string;
  transport: string;
  tools: string[];
  required_scopes: string[];
  installed: boolean;
}

export interface McpToolRef {
  server_id: string;
  tool: string;
}

export interface McpToolGroupRow {
  id: string;
  org_id: string;
  name: string;
  slug: string;
  description: string;
  enabled: boolean;
  tools: McpToolRef[];
  created_at: string;
  updated_at: string;
}

export interface McpGatewaySettingsRow {
  org_id: string;
  default_transport: string;
  connect_timeout_ms: number;
  request_timeout_ms: number;
  max_retries: number;
  default_failure_mode: "fail_open" | "fail_closed";
  allow_unlisted_tools: boolean;
  updated_at: string;
}

export interface McpOAuthGrantRow {
  id: string;
  server_id: string;
  user_id: string;
  scopes: string[];
  granted_at: string;
  revoked_at: string | null;
  revoked_by: string | null;
  /** Resolved server-side so clients never infer state from a nullable timestamp. */
  active: boolean;
}

/** Session metadata for a grant without exposing sealed token material. */
export interface McpOAuthSessionRow {
  id: string;
  grant_id: string;
  scopes: string[];
  expires_at: string;
  refresh_expires_at: string | null;
  revoked_at: string | null;
  created_at: string;
  last_used_at: string | null;
  /** Whether a stored refresh token makes this session renewable. */
  has_refresh_token: boolean;
}

export function fetchMcpServers(orgId: string): Promise<McpServerRow[]> {
  return getJson<McpServerRow[]>(`/api/v1/orgs/${orgId}/mcp-servers`);
}

export function createMcpServer(orgId: string, input: McpServerInput): Promise<McpServerRow> {
  return sendJson<McpServerRow>("POST", `/api/v1/orgs/${orgId}/mcp-servers`, input);
}

export function updateMcpServer(id: string, input: Omit<McpServerInput, "slug" | "source">): Promise<McpServerRow> {
  return sendJson<McpServerRow>("PATCH", `/api/v1/mcp-servers/${id}`, input);
}

export function deleteMcpServer(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/mcp-servers/${id}`);
}

export function fetchMcpLibrary(orgId: string): Promise<McpLibraryItem[]> {
  return getJson<McpLibraryItem[]>(`/api/v1/orgs/${orgId}/mcp/library`);
}

export function fetchMcpToolGroups(orgId: string): Promise<McpToolGroupRow[]> {
  return getJson<McpToolGroupRow[]>(`/api/v1/orgs/${orgId}/mcp/tool-groups`);
}

export function createMcpToolGroup(orgId: string, input: Omit<McpToolGroupRow, "id" | "org_id" | "created_at" | "updated_at">): Promise<McpToolGroupRow> {
  return sendJson<McpToolGroupRow>("POST", `/api/v1/orgs/${orgId}/mcp/tool-groups`, input);
}

export function updateMcpToolGroup(id: string, input: Omit<McpToolGroupRow, "id" | "org_id" | "slug" | "created_at" | "updated_at">): Promise<McpToolGroupRow> {
  return sendJson<McpToolGroupRow>("PUT", `/api/v1/mcp/tool-groups/${id}`, input);
}

export function deleteMcpToolGroup(id: string): Promise<void> {
  return sendJson<void>("DELETE", `/api/v1/mcp/tool-groups/${id}`);
}

export function fetchMcpSettings(orgId: string): Promise<McpGatewaySettingsRow> {
  return getJson<McpGatewaySettingsRow>(`/api/v1/orgs/${orgId}/mcp/settings`);
}

export function updateMcpSettings(orgId: string, input: Omit<McpGatewaySettingsRow, "org_id" | "updated_at">): Promise<McpGatewaySettingsRow> {
  return sendJson<McpGatewaySettingsRow>("PUT", `/api/v1/orgs/${orgId}/mcp/settings`, input);
}

export function fetchMcpGrants(orgId: string): Promise<McpOAuthGrantRow[]> {
  return getJson<McpOAuthGrantRow[]>(`/api/v1/orgs/${orgId}/mcp/grants`);
}

// revoking consent cascades to every session under the grant
export function revokeMcpGrant(id: string): Promise<McpOAuthGrantRow> {
  return sendJson<McpOAuthGrantRow>("DELETE", `/api/v1/mcp/grants/${id}`);
}

export function fetchMcpSessions(orgId: string): Promise<McpOAuthSessionRow[]> {
  return getJson<McpOAuthSessionRow[]>(`/api/v1/orgs/${orgId}/mcp/sessions`);
}

// revoking one session leaves the consent standing; a new session can be
// minted against the same grant without asking the user again
export function revokeMcpSession(id: string): Promise<McpOAuthSessionRow> {
  return sendJson<McpOAuthSessionRow>("DELETE", `/api/v1/mcp/sessions/${id}`);
}

// ---------------------------------------------------------------------------
// complexity routing policy (stored in route params, validated server-side)

export interface ComplexityTier {
  name: string;
  /// inclusive byte ceiling; null marks the final catch-all tier
  max_input_bytes: number | null;
  route: string;
}

export interface ComplexityPolicy {
  tiers: ComplexityTier[];
}

export function fetchRouteComplexity(
  routeId: string,
): Promise<ComplexityPolicy> {
  return getJson<ComplexityPolicy>(`/api/v1/routes/${routeId}/complexity`);
}

export function setRouteComplexity(
  routeId: string,
  policy: ComplexityPolicy,
): Promise<RouteRow> {
  return sendJson<RouteRow>(
    "PUT",
    `/api/v1/routes/${routeId}/complexity`,
    policy,
  );
}

// advanced per-route model configuration (base_url, pricing, limits, headers…)
export function setRouteAdvanced(
  routeId: string,
  advanced: Record<string, unknown>,
): Promise<RouteRow> {
  return sendJson<RouteRow>("PUT", `/api/v1/routes/${routeId}/advanced`, {
    advanced,
  });
}

// ---------------------------------------------------------------------------
// plugin configuration registry (#567; gateway dispatch is tracked by #509)

export interface PluginInstanceRow {
  id: string;
  org_id: string;
  project_id: string | null;
  name: string;
  slug: string;
  description: string;
  kind: "webhook";
  stage: "pre_route" | "pre_upstream" | "post_response";
  enabled: boolean;
  position: number;
  failure_mode: "fail_open" | "fail_closed";
  endpoint: string;
  secret_env: string | null;
  config: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export type PluginInstanceInput = Omit<
  PluginInstanceRow,
  "id" | "org_id" | "slug" | "created_at" | "updated_at"
> & { slug?: string };

export const fetchPlugins = (orgId: string) =>
  getJson<PluginInstanceRow[]>(`/api/v1/orgs/${orgId}/plugins`);
export const createPlugin = (orgId: string, body: PluginInstanceInput) =>
  sendJson<PluginInstanceRow>("POST", `/api/v1/orgs/${orgId}/plugins`, body);
export const updatePlugin = (id: string, body: PluginInstanceInput) =>
  sendJson<PluginInstanceRow>("PUT", `/api/v1/plugins/${id}`, body);
export const deletePlugin = (id: string) =>
  sendJson<void>("DELETE", `/api/v1/plugins/${id}`);

// ---------------------------------------------------------------------------
// deployment-wide guardrail registry

export interface GuardrailRuleRow {
  id: string;
  name: string;
  enabled: boolean;
  source_type: "builtin" | "pattern";
  builtin: "email" | "phone" | "api_token" | "payment_card" | null;
  pattern: string | null;
  stage: "pre_call" | "post_call";
  action: "annotate" | "block" | "redact";
  replacement: string | null;
  include_system: boolean;
  position: number;
  created_at: string;
  updated_at: string;
}

export type GuardrailRuleInput = Omit<GuardrailRuleRow, "id" | "created_at" | "updated_at">;

export interface GuardrailProviderRow {
  id: string;
  name: string;
  enabled: boolean;
  url: string;
  stage: "pre_call" | "post_call";
  timeout_ms: number;
  max_retries: number;
  failure_mode: "fail_open" | "fail_closed";
  max_body_bytes: number;
  auth_kind: "none" | "bearer" | "shared_secret";
  auth_env: string | null;
  created_at: string;
  updated_at: string;
}

export type GuardrailProviderInput = Omit<
  GuardrailProviderRow,
  "id" | "created_at" | "updated_at"
>;

export const fetchGuardrailRules = () =>
  getJson<GuardrailRuleRow[]>("/api/v1/guardrails/rules");
export const createGuardrailRule = (body: GuardrailRuleInput) =>
  sendJson<GuardrailRuleRow>("POST", "/api/v1/guardrails/rules", body);
export const updateGuardrailRule = (id: string, body: GuardrailRuleInput) =>
  sendJson<GuardrailRuleRow>("PUT", `/api/v1/guardrails/rules/${id}`, body);
export const deleteGuardrailRule = (id: string) =>
  sendJson<void>("DELETE", `/api/v1/guardrails/rules/${id}`);

export const fetchGuardrailProviders = () =>
  getJson<GuardrailProviderRow[]>("/api/v1/guardrails/providers");
export const createGuardrailProvider = (body: GuardrailProviderInput) =>
  sendJson<GuardrailProviderRow>("POST", "/api/v1/guardrails/providers", body);
export const updateGuardrailProvider = (id: string, body: GuardrailProviderInput) =>
  sendJson<GuardrailProviderRow>("PUT", `/api/v1/guardrails/providers/${id}`, body);
export const deleteGuardrailProvider = (id: string) =>
  sendJson<void>("DELETE", `/api/v1/guardrails/providers/${id}`);
