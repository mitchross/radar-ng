import { telemetryExporterConfig } from "../../src/lib/telemetryExport";

describe("telemetry export target", () => {
  it("posts each signal to the stack's relay", () => {
    expect(telemetryExporterConfig("https://radar.example.com/", "traces").url).toBe(
      "https://radar.example.com/api/telemetry/v1/traces",
    );
    expect(telemetryExporterConfig("https://radar.example.com", "logs").url).toBe(
      "https://radar.example.com/api/telemetry/v1/logs",
    );
  });

  it("adds the bearer header only when a token is set", () => {
    expect(telemetryExporterConfig("https://r.example", "logs").headers).toEqual({});
    expect(telemetryExporterConfig("https://r.example", "logs", "").headers).toEqual({});
    expect(telemetryExporterConfig("https://r.example", "logs", "abc").headers).toEqual({
      Authorization: "Bearer abc",
    });
  });
});
