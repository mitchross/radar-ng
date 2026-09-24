import { absolutizeStyle, DEFAULT_LABEL_FONT, labelFontFor } from "../../src/lib/basemapStyle";

describe("absolutizeStyle", () => {
  it("rewrites relative tiles, glyphs and sprite onto the server and leaves absolute URLs alone", () => {
    const style = absolutizeStyle(
      {
        glyphs: "/basemap/fonts/{fontstack}/{range}.pbf",
        sprite: "/basemap/sprites/basics",
        sources: {
          basemap: { tiles: ["/basemap/tiles/{z}/{x}/{y}.mvt"] },
          other: { url: "https://maps.example/tiles.json" },
        },
      },
      "https://radar.example",
    );

    expect(style.glyphs).toBe("https://radar.example/basemap/fonts/{fontstack}/{range}.pbf");
    expect(style.sprite).toBe("https://radar.example/basemap/sprites/basics");
    expect(style.sources?.basemap.tiles).toEqual(["https://radar.example/basemap/tiles/{z}/{x}/{y}.mvt"]);
    expect(style.sources?.other.url).toBe("https://maps.example/tiles.json");
  });
});

describe("labelFontFor", () => {
  it("reuses a regular fontstack the basemap already serves (VersaTiles naming)", () => {
    const font = labelFontFor({
      layers: [
        { layout: { "text-font": ["noto_sans_bold"] } },
        { layout: { "text-font": ["noto_sans_regular"] } },
      ],
    });
    expect(font).toEqual(["noto_sans_regular"]);
  });

  it("skips data-driven text-font expressions", () => {
    expect(
      labelFontFor({ layers: [{ layout: { "text-font": ["literal", ["x"]] as unknown } }, { layout: { "text-font": ["Noto Sans Regular"] } }] }),
    ).toEqual(["Noto Sans Regular"]);
  });

  it("falls back to the bundled fontstack before a style has loaded", () => {
    expect(labelFontFor(undefined)).toEqual(DEFAULT_LABEL_FONT);
  });
});
