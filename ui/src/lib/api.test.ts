import { describe, it, expect, mock, beforeEach, afterEach } from "bun:test";
import {
  fetchConfig,
  createOrg,
  fetchAnalyticsSummary,
  AnalyticsUnavailableError,
  apiBaseDoublesV1,
  resolveUpstreamUrl,
} from "./api";

// We need to mock global fetch and localStorage
describe("api client", () => {
  let fetchMock: any;
  let localStorageMock: Record<string, string>;

  beforeEach(() => {
    fetchMock = mock();
    globalThis.fetch = fetchMock;

    localStorageMock = {};
    const getItemMock = mock((key: string) => localStorageMock[key] || null);
    const setItemMock = mock((key: string, val: string) => {
      localStorageMock[key] = val;
    });

    Object.defineProperty(globalThis, "localStorage", {
      value: {
        getItem: getItemMock,
        setItem: setItemMock,
      },
      writable: true,
    });
  });

  afterEach(() => {
    mock.restore();
  });

  describe("getJson (via fetchConfig)", () => {
    it("should make a GET request with auth headers if token exists", async () => {
      localStorageMock["rolter.session.token"] = "test-token";
      fetchMock.mockResolvedValueOnce(
        new Response(
          JSON.stringify({ providers: [], routes: [], virtual_keys: [] }),
          {
            status: 200,
          },
        ),
      );

      const result = await fetchConfig();
      expect(result).toEqual({ providers: [], routes: [], virtual_keys: [] });
      expect(fetchMock).toHaveBeenCalledTimes(1);

      const callArgs = fetchMock.mock.calls[0];
      expect(callArgs[0]).toBe("/api/v1/config");
      expect(callArgs[1].headers).toEqual({
        Authorization: "Bearer test-token",
      });
    });

    it("should make a GET request without auth headers if no token exists", async () => {
      fetchMock.mockResolvedValueOnce(
        new Response(
          JSON.stringify({ providers: [], routes: [], virtual_keys: [] }),
          {
            status: 200,
          },
        ),
      );

      await fetchConfig();
      const callArgs = fetchMock.mock.calls[0];
      expect(callArgs[1].headers).toEqual({});
    });

    it("should throw an error on non-ok status (testing apiError without json)", async () => {
      fetchMock.mockResolvedValueOnce(
        new Response("Not Found", {
          status: 404,
        }),
      );

      await expect(fetchConfig()).rejects.toThrow("request failed: 404");
    });

    it("should surface control plane error messages", async () => {
      fetchMock.mockResolvedValueOnce(
        new Response(
          JSON.stringify({ error: { message: "superadmin access required" } }),
          {
            status: 403,
          },
        ),
      );

      const request = fetchConfig();
      await expect(request).rejects.toThrow("superadmin access required");
      await expect(request).rejects.toMatchObject({ status: 403 });
    });
  });

  describe("sendJson (via createOrg)", () => {
    it("should make a POST request with JSON body and correct headers", async () => {
      fetchMock.mockResolvedValueOnce(
        new Response(JSON.stringify({ id: "org-1", name: "Test Org" }), {
          status: 200,
        }),
      );

      const input = { name: "Test Org", slug: "test-org" };
      const result = await createOrg(input);

      expect(result.id).toBe("org-1");
      const callArgs = fetchMock.mock.calls[0];
      expect(callArgs[0]).toBe("/api/v1/orgs");
      expect(callArgs[1].method).toBe("POST");
      expect(callArgs[1].headers).toHaveProperty(
        "Content-Type",
        "application/json",
      );
      expect(callArgs[1].body).toBe(JSON.stringify(input));
    });

    it("should parse and throw control plane API errors", async () => {
      fetchMock.mockResolvedValueOnce(
        new Response(
          JSON.stringify({ error: { message: "Invalid org name" } }),
          {
            status: 400,
          },
        ),
      );

      await expect(createOrg({ name: "Bad", slug: "bad" })).rejects.toThrow(
        "Invalid org name",
      );
    });
  });

  describe("getAnalytics (via fetchAnalyticsSummary)", () => {
    it("should fetch analytics data successfully", async () => {
      const summaryData = {
        requests: 100,
        cost_usd: 5.5,
        tokens: 10,
        prompt_tokens: 5,
        completion_tokens: 5,
        failures: 0,
        p95_latency_ms: 100,
        cached_requests: 0,
        cache_hits: 0,
        avg_latency_ms: 50,
        errors: 0,
        unpriced_requests: 0,
        unpriced_models: 0,
      };
      fetchMock.mockResolvedValueOnce(
        new Response(JSON.stringify({ data: [summaryData] }), {
          status: 200,
        }),
      );

      const result = await fetchAnalyticsSummary();
      expect(result).toEqual(summaryData);
      const callArgs = fetchMock.mock.calls[0];
      expect(callArgs[0]).toContain("/api/v1/analytics/summary");
    });

    it("should throw AnalyticsUnavailableError on 503", async () => {
      fetchMock.mockResolvedValueOnce(
        new Response(
          JSON.stringify({ error: { message: "ClickHouse down" } }),
          {
            status: 503,
          },
        ),
      );

      await expect(fetchAnalyticsSummary()).rejects.toThrow(
        AnalyticsUnavailableError,
      );
    });

    it("should throw AnalyticsUnavailableError on 404 by default", async () => {
      fetchMock.mockResolvedValueOnce(
        new Response("Not found", {
          status: 404,
        }),
      );

      await expect(fetchAnalyticsSummary()).rejects.toThrow(
        AnalyticsUnavailableError,
      );
    });
  });
});

// #947: the base-URL field taught operators to include /v1, which for
// openai-shaped kinds doubles into /v1/v1/chat/completions and 404s.
describe("api_base resolution", () => {
  it("appends /v1 for kinds that do not carry it in the base", () => {
    expect(resolveUpstreamUrl("https://api.openai.com", false)).toBe(
      "https://api.openai.com/v1/chat/completions",
    );
  });

  it("reproduces the doubling the old placeholder caused", () => {
    // the literal base from the dogfooding report
    expect(resolveUpstreamUrl("https://gpustack.localhost/v1", false)).toBe(
      "https://gpustack.localhost/v1/v1/chat/completions",
    );
    expect(apiBaseDoublesV1("https://gpustack.localhost/v1", false)).toBe(true);
  });

  it("strips the gateway /v1 for kinds whose base carries it", () => {
    expect(resolveUpstreamUrl("https://api.mistral.ai/v1", true)).toBe(
      "https://api.mistral.ai/v1/chat/completions",
    );
    // the same spelling is correct here, so it is not flagged
    expect(apiBaseDoublesV1("https://api.mistral.ai/v1", true)).toBe(false);
  });

  it("does not double the separator on a trailing slash", () => {
    expect(resolveUpstreamUrl("https://host/", false)).toBe(
      "https://host/v1/chat/completions",
    );
    expect(apiBaseDoublesV1("https://host/v1/", false)).toBe(true);
  });

  it("previews nothing for an empty base", () => {
    expect(resolveUpstreamUrl("", false)).toBe("");
    expect(apiBaseDoublesV1("", false)).toBe(false);
  });

  it("does not mistake /v1beta for the version prefix", () => {
    expect(apiBaseDoublesV1("https://host/v1beta", false)).toBe(false);
  });
});
