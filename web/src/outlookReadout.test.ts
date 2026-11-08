import { afterEach, expect, it, vi } from "vitest";
import { pointOutlookHtml } from "./outlookReadout";

afterEach(() => vi.unstubAllGlobals());

it("reads Days 1-3 from our server and preserves failed-day availability", async () => {
  const fetcher = vi.fn(async (url: string) => ({
    ok: !url.endsWith("2"),
    json: async () => ({ type: "FeatureCollection", features: [{ type: "Feature", geometry: { type: "Polygon", coordinates: [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]] }, properties: { label: "SLGT", name: "<script>unsafe</script>", expire: "2099-01-01T00:00:00Z" } }] }),
  }));
  vi.stubGlobal("fetch", fetcher);
  const html = await pointOutlookHtml(1, 1);
  expect(fetcher.mock.calls.map(([url]) => url)).toEqual(["/api/outlooks?day=1", "/api/outlooks?day=2", "/api/outlooks?day=3"]);
  expect(html).toContain("Day 1: SLGT");
  expect(html).toContain("Day 2: Outlook unavailable");
  expect(html).toContain("Day 3: SLGT");
  expect(html).not.toContain("<script>");
  expect(html).toContain("&lt;script&gt;");
});
