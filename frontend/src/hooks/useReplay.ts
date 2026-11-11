import { useQuery } from "@tanstack/react-query";
import { useWeatherStore } from "../stores/useWeatherStore";
import { isReplaySource } from "../lib/replaySource";
import { replayTileTemplate } from "../lib/replayApi";
import {
  fetchReplayCatalogue,
  fetchReplayManifest,
  fetchReplayWarnings,
  replayCatalogueKey,
  replayManifestKey,
  replayWarningsKey,
} from "../lib/replayClient";

export const useIsReplay = (): boolean => useWeatherStore((s) => isReplaySource(s.source));

/** The catalogue behind the Famous storms sheet; errors surface, never a live fallback. */
export function useReplayCatalogue(enabled: boolean) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  return useQuery({
    queryKey: replayCatalogueKey(serverUrl),
    queryFn: ({ signal }) => fetchReplayCatalogue(serverUrl, signal),
    enabled,
    retry: false,
    staleTime: 5 * 60_000,
  });
}

/** The case manifest, only while replaying; never reads or writes the live manifest cache. */
export function useReplayManifestQuery() {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const source = useWeatherStore((s) => s.source);
  const replayCase = useWeatherStore((s) => s.replayCase);
  return useQuery({
    queryKey: replayManifestKey(serverUrl, source),
    queryFn: ({ signal }) => fetchReplayManifest(serverUrl, replayCase!, signal),
    enabled: isReplaySource(source) && replayCase != null,
    retry: false,
    // A revision is immutable.
    staleTime: Infinity,
  });
}

/** Tile template of the active case, or null until its manifest has loaded. */
export function useReplayTileTemplate(): string | null {
  const manifest = useReplayManifestQuery().data;
  return manifest ? replayTileTemplate(manifest) : null;
}

/** Warning history of the active case; the playhead filters it, nothing polls. */
export function useReplayWarningsQuery() {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const source = useWeatherStore((s) => s.source);
  const replayCase = useWeatherStore((s) => s.replayCase);
  return useQuery({
    queryKey: replayWarningsKey(serverUrl, source),
    queryFn: ({ signal }) => fetchReplayWarnings(serverUrl, replayCase!, signal),
    enabled: isReplaySource(source) && replayCase != null,
    retry: false,
    staleTime: Infinity,
  });
}
