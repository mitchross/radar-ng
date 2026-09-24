import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchWithTimeout } from "../lib/api";
import { absolutizeStyle, labelFontFor, type StyleDocument } from "../lib/basemapStyle";
import { isExternalMapStyle, resolveMapStyleUrl, type MapStyleId } from "../lib/constants";
import { trace } from "../lib/telemetry";

/**
 * The active basemap style, fetched once per URL and shared by every map
 * (radar tab, home mini-map) and by overlays that need its fonts.
 *
 * External styles (e.g. the self-hosted VersaTiles) are complete absolute
 * documents, so MapLibre gets the URL immediately; the document is still
 * fetched in the background to learn its label fonts. Bundled styles have
 * server-relative paths and must be rewritten before MapLibre sees them.
 */
export function useBasemapStyle(serverUrl: string, mapStyle: MapStyleId) {
  const styleUrl = resolveMapStyleUrl(serverUrl, mapStyle);
  const external = isExternalMapStyle(mapStyle);

  const query = useQuery({
    queryKey: ["map-style", styleUrl, external ? null : serverUrl],
    queryFn: ({ signal }) =>
      trace(
        "map.fetchStyle",
        async () => {
          const res = await fetchWithTimeout(styleUrl, {}, signal);
          if (!res.ok) throw new Error(`Style error: ${res.status}`);
          const style = (await res.json()) as StyleDocument;
          return external ? style : absolutizeStyle(style, serverUrl);
        },
        { "map.style": mapStyle },
      ),
    staleTime: Infinity,
    gcTime: 24 * 60 * 60_000,
    // Android can fire this before its network stack is up at cold start.
    retry: 3,
    retryDelay: (attempt) => 400 * 2 ** attempt,
  });

  const style = useMemo<string | null>(() => {
    if (external) return styleUrl;
    if (query.data) return JSON.stringify(query.data);
    // Every retry failed: hand MapLibre the raw URL so attribution and
    // controls still render, even though relative tile paths won't resolve.
    if (query.isError) return styleUrl;
    return null;
  }, [external, styleUrl, query.data, query.isError]);

  const labelFont = useMemo(() => labelFontFor(query.data), [query.data]);

  return { style, labelFont };
}
