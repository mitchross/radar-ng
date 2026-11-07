import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { LAYER_PALETTE_KEY } from "./serverPalettes";

const table = (id: string) =>
  JSON.parse(readFileSync(new URL(`../../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8")) as Record<string, any>;
const keys = (t: Record<string, unknown>) => Object.keys(t).filter((k) => !k.startsWith("_"));

describe("muted (colour-blind safe) palette coverage", () => {
  it("has a table for every colour key the other palettes define", () => {
    const muted = keys(table("muted"));
    for (const id of ["classic", "vivid"]) expect(muted, id).toEqual(expect.arrayContaining(keys(table(id))));
  });

  it("has banded ranges for every layer the clients draw a legend for", () => {
    const muted = table("muted");
    for (const [layer, key] of Object.entries(LAYER_PALETTE_KEY)) {
      expect(muted[key]?.ranges?.length, `${layer} -> muted.${key}`).toBeGreaterThan(0);
    }
  });
});
