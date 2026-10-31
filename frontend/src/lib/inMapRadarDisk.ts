/**
 * File-system side of the in-map radar engine (see inMapRadarTiles.ts for the layout). Thin
 * wrappers over expo-file-system's File/Directory API, which ships with `expo` (autolinked), so no
 * extra dependency. Sizes are tracked by the caller from the bytes it writes; nothing here walks tiles.
 */
import { Directory, File, Paths } from "expo-file-system";
import { errorMessage, recordInMapError } from "./inMapDiag";
import { inMapRootUri, IN_MAP_DIR, tilePath, type DiskFrameListing } from "./inMapRadarTiles";
import type { TileId } from "./packed";

const root = () => new Directory(Paths.cache, IN_MAP_DIR);

/** `file://` root with a trailing slash, the prefix of every MapLibre tile URL template. */
export const rootUri = () => inMapRootUri(Paths.cache.uri);

const tileFile = (layer: string, frameKey: string, gen: number, t: TileId) =>
  new File(root(), tilePath(layer, frameKey, gen, t));

export async function writeTile(layer: string, frameKey: string, gen: number, t: TileId, bytes: Uint8Array) {
  const file = tileFile(layer, frameKey, gen, t);
  file.create({ intermediates: true, overwrite: true });
  await file.write(bytes);
}

/** Copy a tile from one gen to another (a reused tile); false if the source is gone. */
export async function copyTile(layer: string, frameKey: string, fromGen: number, toGen: number, t: TileId) {
  const from = tileFile(layer, frameKey, fromGen, t);
  if (!from.exists) return false;
  const to = tileFile(layer, frameKey, toGen, t);
  to.create({ intermediates: true, overwrite: true });
  await from.copy(to, { overwrite: true });
  return true;
}

/** Where a tile is and whether it is on disk with some bytes: the engine's check before a gen is committed. */
export function verifyTile(layer: string, frameKey: string, gen: number, t: TileId): { uri: string; exists: boolean } {
  const file = tileFile(layer, frameKey, gen, t);
  return { uri: file.uri, exists: file.exists && (file.size ?? 1) > 0 };
}

/** Delete paths relative to the root (a frame dir or a gen dir); missing ones are ignored. */
export function deleteRelative(paths: readonly string[]) {
  for (const rel of paths) {
    try {
      const dir = new Directory(root(), rel);
      if (dir.exists) dir.delete();
    } catch (err) {
      recordInMapError("write", `cleanup: ${errorMessage(err)}`);
    }
  }
}

/** Remove everything the engine wrote. */
export function wipeAll() {
  try {
    const dir = root();
    if (dir.exists) dir.delete();
  } catch (err) {
    recordInMapError("write", `wipe: ${errorMessage(err)}`);
  }
}

/** Every `layer/frameKey/gen` directory on disk, grouped by frame. */
export function listFrames(): DiskFrameListing[] {
  const out: DiskFrameListing[] = [];
  try {
    const top = root();
    if (!top.exists) return out;
    for (const layer of top.list()) {
      if (!(layer instanceof Directory)) continue;
      for (const frame of layer.list()) {
        if (!(frame instanceof Directory)) continue;
        const gens = frame
          .list()
          .filter((g): g is Directory => g instanceof Directory)
          .map((g) => Number(g.name))
          .filter(Number.isInteger);
        out.push({ layer: layer.name, frameKey: frame.name, gens });
      }
    }
  } catch (err) {
    recordInMapError("write", `list: ${errorMessage(err)}`);
  }
  return out;
}
