import { shouldPersistQuery } from "../../src/lib/queryPersistence";

const query = (queryKey: unknown[], status: "success" | "error" | "pending") =>
  ({ queryKey, state: { status } }) as unknown as Parameters<typeof shouldPersistQuery>[0];

describe("shouldPersistQuery", () => {
  it("saves successful forecast, alerts and radar nowcast results", () => {
    expect(shouldPersistQuery(query(["forecast", "42.96,-85.67", "https://x"], "success"))).toBe(true);
    expect(shouldPersistQuery(query(["alerts", "42.963,-85.668", "https://x"], "success"))).toBe(true);
    expect(shouldPersistQuery(query(["radar-nowcast", "k", "https://x"], "success"))).toBe(true);
  });

  it("does not save failures or high-churn families", () => {
    expect(shouldPersistQuery(query(["forecast", "k"], "error"))).toBe(false);
    expect(shouldPersistQuery(query(["manifest", "https://x"], "success"))).toBe(false);
    expect(shouldPersistQuery(query(["lightning", "https://x"], "success"))).toBe(false);
  });
});
