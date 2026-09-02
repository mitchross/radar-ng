import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchStormPrefetchPlan } from "../lib/api";
import { logEvent } from "../lib/telemetry";
import { telemetryErrorType } from "../lib/telemetryPrivacy";
import { useWeatherStore } from "../stores/useWeatherStore";

const PREFETCH_ZOOM = 6;
const PACK_KIND = "radar-ng-storm-prefetch";

/**
 * Loads three small storm-track regions through MapLibre's native offline
 * manager. A JS fetch/Image.prefetch would populate a different cache and
 * would not help the RasterSource on iOS.
 */
export function useStormTilePrefetch() {
  const serverUrl = useWeatherStore((state) => state.serverUrl);
  const latitude = useWeatherStore((state) => state.latitude);
  const longitude = useWeatherStore((state) => state.longitude);
  const activePalette = useWeatherStore((state) => state.activePalette);
  const extrasVisible = useWeatherStore((state) => state.extrasVisible);

  const query = useQuery({
    queryKey: ["storm-prefetch", serverUrl, latitude, longitude, activePalette],
    queryFn: ({ signal }) => fetchStormPrefetchPlan(
      serverUrl,
      latitude as number,
      longitude as number,
      activePalette,
      PREFETCH_ZOOM,
      signal,
    ),
    enabled: extrasVisible && latitude != null && longitude != null,
    staleTime: 5 * 60_000,
    refetchInterval: extrasVisible ? 5 * 60_000 : false,
  });

  useEffect(() => {
    if (!extrasVisible) return;
    const plan = query.data;
    if (!plan?.plan_id || plan.bboxes.length !== 3) return;
    const currentPlan = plan;
    let cancelled = false;

    async function syncNativePacks() {
      // Dynamic import keeps the web bundle and unit-test environment from
      // eagerly initializing MapLibre's native TurboModule.
      const { OfflineManager } = await import("@maplibre/maplibre-react-native");
      const existing = await OfflineManager.getPacks();
      const matching = existing.filter(
        (pack) => pack.metadata.kind === PACK_KIND && pack.metadata.planId === currentPlan.plan_id,
      );
      const matchingLeads = new Set(matching.map((pack) => pack.metadata.leadMinutes));
      const regionsToCreate = currentPlan.bboxes.filter(
        (region) =>
          region.style_url &&
          region.tile_urls.length > 0 &&
          !matchingLeads.has(region.lead_minutes),
      );

      if (regionsToCreate.length > 0) {
        await Promise.all(
          regionsToCreate.map((region) => OfflineManager.createPack(
              {
                mapStyle: region.style_url as string,
                bounds: region.bbox,
                minZoom: region.zoom,
                maxZoom: region.zoom,
                metadata: {
                  kind: PACK_KIND,
                  planId: currentPlan.plan_id,
                  leadMinutes: region.lead_minutes,
                },
              },
              () => {},
              (_pack, error) => {
                logEvent("warn", "storm tile prefetch failed", {
                  "error.type": telemetryErrorType(error),
                  "prefetch.lead_minutes": region.lead_minutes,
                });
              },
            )),
        );
      }

      if (cancelled) return;
      await Promise.all(
        existing
          .filter((pack) => pack.metadata.kind === PACK_KIND && pack.metadata.planId !== currentPlan.plan_id)
          .map((pack) => OfflineManager.deletePack(pack.id)),
      );
    }

    syncNativePacks().catch((error: unknown) => {
      logEvent("warn", "storm tile prefetch unavailable", {
        "error.type": telemetryErrorType(error),
      });
    });
    return () => {
      cancelled = true;
    };
  }, [extrasVisible, query.data]);

  return query;
}
