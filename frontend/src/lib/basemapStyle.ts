/**
 * Pure helpers for the basemap style document. Kept free of React so the
 * URL rewriting and font choice can be unit-tested directly.
 */

export interface StyleDocument {
  glyphs?: string;
  sprite?: string | { id: string; url: string }[];
  sources?: Record<string, { tiles?: string[]; url?: string }>;
  layers?: { layout?: Record<string, unknown> }[];
}

const absolute = (value: string, serverUrl: string) =>
  value.startsWith("http") ? value : `${serverUrl}${value}`;

/**
 * The bundled styles use server-relative paths (`/basemap/tiles/...`,
 * `/basemap/fonts/...`) so one image works under any hostname. MapLibre Native
 * does not resolve those against the style URL, so rewrite them to absolute.
 */
export function absolutizeStyle(style: StyleDocument, serverUrl: string): StyleDocument {
  const sources = Object.fromEntries(
    Object.entries(style.sources ?? {}).map(([id, src]) => [
      id,
      {
        ...src,
        ...(Array.isArray(src.tiles) ? { tiles: src.tiles.map((t) => absolute(t, serverUrl)) } : {}),
        ...(typeof src.url === "string" ? { url: absolute(src.url, serverUrl) } : {}),
      },
    ]),
  );
  return {
    ...style,
    sources,
    ...(typeof style.glyphs === "string" ? { glyphs: absolute(style.glyphs, serverUrl) } : {}),
    ...(typeof style.sprite === "string" ? { sprite: absolute(style.sprite, serverUrl) } : {}),
  };
}

/** Fontstack of the bundled Protomaps styles; also the fallback before a style loads. */
export const DEFAULT_LABEL_FONT = ["Noto Sans Regular"];

/**
 * A fontstack the active basemap's glyph server actually serves. Overlay labels
 * must reuse one: VersaTiles names fonts `noto_sans_regular` while Protomaps
 * uses `Noto Sans Regular`, and an unknown fontstack silently drops the label.
 */
export function labelFontFor(style: StyleDocument | undefined): string[] {
  const stacks: string[][] = [];
  for (const layer of style?.layers ?? []) {
    const font = layer.layout?.["text-font"];
    if (Array.isArray(font) && font.length > 0 && font.every((f) => typeof f === "string")) {
      stacks.push(font as string[]);
    }
  }
  return stacks.find((s) => /regular/i.test(s[0])) ?? stacks[0] ?? DEFAULT_LABEL_FONT;
}
