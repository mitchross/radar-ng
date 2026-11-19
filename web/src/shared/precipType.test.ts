import { describe, expect, it } from "vitest";
import { ICE_NOTE, precipClassNote } from "./precipType";

describe("precipClassNote", () => {
  it("names ice only when the server marks the point as ice", () => {
    expect(precipClassNote({ precip_class: "ice" })).toBe(ICE_NOTE);
    expect(ICE_NOTE).toBe("Ice: freezing rain or sleet, from HRRR model guidance");
    expect(precipClassNote({ precip_class: "snow" })).toBeNull();
    expect(precipClassNote({})).toBeNull();
    expect(precipClassNote(null)).toBeNull();
  });
});
