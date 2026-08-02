// design-prototype mock data for the remaining screens whose backend DTOs do
// not exist yet. each consumer labels itself as preview data; MCP Catalog was
// removed from this file when the real registry API landed.

export interface RbacResource {
  key: string;
  label: string;
}

export const RBAC_RESOURCES: RbacResource[] = [
  { key: "virtual_keys", label: "Virtual Keys" },
  { key: "providers", label: "Providers" },
  { key: "budgets", label: "Budgets & Limits" },
  { key: "teams", label: "Teams & Customers" },
  { key: "logs", label: "Logs & Analytics" },
  { key: "settings", label: "Settings" },
  { key: "rbac", label: "Roles & Permissions" },
];

export interface RbacRole {
  key: string;
  label: string;
  members: number;
  desc: string;
  // per-resource op string out of "vcud" (view/create/update/delete)
  caps: Record<string, string>;
}

export const RBAC_ROLES: RbacRole[] = [
  {
    key: "admin",
    label: "Admin",
    members: 1,
    desc: "Manage gateway config, keys, and governance across the org.",
    caps: { virtual_keys: "vcud", providers: "vcud", budgets: "vcud", teams: "vcud", logs: "vc", settings: "vu", rbac: "vu" },
  },
  {
    key: "member",
    label: "Member",
    members: 0,
    desc: "Use the gateway and manage their own virtual keys.",
    caps: { virtual_keys: "vcu", providers: "v", budgets: "v", teams: "v", logs: "v", settings: "", rbac: "" },
  },
  {
    key: "viewer",
    label: "Viewer",
    members: 0,
    desc: "Read-only access to dashboards, logs, and config.",
    caps: { virtual_keys: "v", providers: "v", budgets: "v", teams: "v", logs: "v", settings: "", rbac: "" },
  },
];

export interface FeatureFlag {
  key: string;
  label: string;
  desc: string;
  on: boolean;
}

export const FEATURE_FLAGS: FeatureFlag[] = [
  { key: "cache", label: "Response cache", desc: "Direct + semantic response caching", on: true },
  { key: "cache_aware", label: "Cache-aware routing", desc: "Prefer targets with a warm KV cache", on: true },
  { key: "circuit", label: "Circuit breaker", desc: "Fail over when an endpoint degrades", on: true },
  { key: "health", label: "Active health checks", desc: "Probe upstreams on an interval", on: true },
  { key: "complexity", label: "Complexity router", desc: "Route by prompt complexity tier", on: false },
  { key: "guardrails", label: "Guardrails", desc: "Input/output safety checks", on: false },
];
