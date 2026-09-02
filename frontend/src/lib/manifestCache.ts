import type { SelfHostedManifest } from "../types/weather";
import { getString, setString } from "./storage";
import { tryParseSelfHostedManifest } from "./manifest";

export const MANIFEST_CACHE_KEY = "manifest-cache-v2";

interface ManifestCacheEnvelope {
  serverUrl: string;
  manifest: SelfHostedManifest;
}

function readEnvelope(): ManifestCacheEnvelope | undefined {
  const cached = getString(MANIFEST_CACHE_KEY, "");
  if (!cached) return undefined;

  try {
    const parsed: unknown = JSON.parse(cached);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return undefined;
    const envelope = parsed as Record<string, unknown>;
    if (typeof envelope.serverUrl !== "string") return undefined;
    const manifest = tryParseSelfHostedManifest(envelope.manifest);
    return manifest ? { serverUrl: envelope.serverUrl, manifest } : undefined;
  } catch {
    return undefined;
  }
}

export function readCachedManifest(serverUrl: string): SelfHostedManifest | undefined {
  const cached = readEnvelope();
  return cached?.serverUrl === serverUrl ? cached.manifest : undefined;
}

/** Returns true only when a storage write was needed. */
export function cacheManifestIfChanged(serverUrl: string, value: SelfHostedManifest): boolean {
  const cached = readEnvelope();
  if (cached?.serverUrl === serverUrl && cached.manifest.updated_at === value.updated_at) {
    return false;
  }

  // `value` crossed the runtime validator in fetchSelfHostedManifest. Avoid a
  // second full walk of every frame before writing the validated object.
  setString(MANIFEST_CACHE_KEY, JSON.stringify({ serverUrl, manifest: value }));
  return true;
}
