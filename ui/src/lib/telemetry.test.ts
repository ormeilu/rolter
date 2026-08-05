import { describe, it, expect, beforeEach } from "bun:test";
import { initTelemetry, resetTelemetryForTests } from "./telemetry";

describe("browser telemetry", () => {
  beforeEach(() => {
    resetTelemetryForTests();
  });

  it("stays off when no runtime config was injected", async () => {
    // the default for every deployment: the control plane injected nothing, so
    // the dashboard must load no SDK and export nothing
    expect(await initTelemetry(undefined)).toBe(false);
  });

  it("stays off when the config carries no endpoint", async () => {
    // a config block exists (the control plane always writes one) but
    // observability is not configured on this deployment
    expect(await initTelemetry({})).toBe(false);
    expect(await initTelemetry({ otelServiceName: "rolter-ui" })).toBe(false);
  });

  it("stays off outside a browser even with an endpoint", async () => {
    // the web instrumentations touch window/document when constructed. this
    // runs without a DOM, so the guard is what keeps it from throwing — the
    // same guard that protects any future server-side render
    expect(
      await initTelemetry({ otelEndpoint: "http://localhost:4318/v1/traces" }),
    ).toBe(false);
  });
});
