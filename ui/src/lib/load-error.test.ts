import { describe, expect, it } from "bun:test";

import { ApiError } from "@/lib/api";
import { classifyLoadError, isRetryable, needsSignIn } from "@/lib/load-error";

describe("classifyLoadError", () => {
  // the #942 case that motivated #962: every /api/v1/me/* route returned 401
  // while the Keys screen said "Failed to load your keys.", sending the
  // operator to check key configuration
  it("reads a 401 as an auth problem, not a data problem", () => {
    expect(classifyLoadError(new ApiError("unauthorized", 401))).toBe("unauthenticated");
  });

  it("separates open mode from a plain 401", () => {
    const openMode = new ApiError("no session", 401, "open_mode_no_session");
    expect(classifyLoadError(openMode)).toBe("openMode");
    // and the fix is not the same one
    expect(needsSignIn(classifyLoadError(openMode))).toBe(false);
    expect(needsSignIn(classifyLoadError(new ApiError("nope", 401)))).toBe(true);
  });

  it("reads a 403 as a permission problem", () => {
    expect(classifyLoadError(new ApiError("forbidden", 403))).toBe("forbidden");
  });

  it("reads a 5xx as the server failing", () => {
    expect(classifyLoadError(new ApiError("boom", 500))).toBe("server");
    expect(classifyLoadError(new ApiError("boom", 503))).toBe("server");
  });

  // fetch rejects with a TypeError when it cannot connect, so an error that
  // carries no status never reached the control plane at all
  it("reads a non-ApiError as never having reached the control plane", () => {
    expect(classifyLoadError(new TypeError("Failed to fetch"))).toBe("unreachable");
    expect(classifyLoadError(undefined)).toBe("unreachable");
  });

  it("falls back to unknown for an unanticipated 4xx", () => {
    expect(classifyLoadError(new ApiError("teapot", 418))).toBe("unknown");
  });
});

describe("isRetryable", () => {
  // offering a retry that cannot work suggests the failure was transient when
  // it was a permission or a deployment setting
  it("does not offer retry where retrying cannot help", () => {
    expect(isRetryable("unauthenticated")).toBe(false);
    expect(isRetryable("forbidden")).toBe(false);
    expect(isRetryable("openMode")).toBe(false);
  });

  it("offers retry where the failure could be transient", () => {
    expect(isRetryable("unreachable")).toBe(true);
    expect(isRetryable("server")).toBe(true);
    expect(isRetryable("unknown")).toBe(true);
  });
});

// a store-less control plane 404s every CRUD route: that is deployment shape,
// and neither a retry nor a sign-in can change it (#1204)
describe("noStore", () => {
  it("a control-plane 404 carrying the endpoint code classifies as noStore", () => {
    const byCode = new ApiError("no such endpoint: /api/v1/orgs", 404, "no_such_endpoint");
    expect(classifyLoadError(byCode)).toBe("noStore");
    expect(isRetryable("noStore")).toBe(false);
    expect(needsSignIn("noStore")).toBe(false);
  });

  it("an older control plane is recognised by the message alone", () => {
    const byMessage = new ApiError("no such endpoint: /api/v1/orgs", 404);
    expect(classifyLoadError(byMessage)).toBe("noStore");
  });

  it("any other 404 stays unknown", () => {
    expect(classifyLoadError(new ApiError("not found", 404))).toBe("unknown");
  });
});
