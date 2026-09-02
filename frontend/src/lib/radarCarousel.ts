/**
 * Slot assignment for the raster frame carousel (RasterFrameCarousel.tsx).
 *
 * The overlay mounts a CONSTANT number of raster sources ("slots"). A
 * mounted MapLibre RasterSource cannot change its tile URL in place
 * (maplibre-react-native 11.x: MLRNRasterSource only implements makeSource),
 * so advancing a frame means remounting one source — the trick is to only
 * ever remount a HIDDEN slot, several ticks before it becomes visible, so
 * its tiles are already fetched when its opacity flips on.
 *
 * Assignment rule: the N slots hold the next N frames in playback order
 * starting at the current frame, wrapping inside the playback window.
 * Positions are taken modulo a window length padded up to a multiple of N,
 * which guarantees each of the N consecutive positions lands in a distinct
 * slot AND that a +1 advance changes exactly one slot (the one that just
 * played, which becomes the farthest prefetch). Padding aliases positions
 * past the window end back to the window start, so the loop wrap is
 * prefetched like any other frame. When the window is shorter than N,
 * some slots hold duplicate frames — harmless, they stay hidden.
 */

/**
 * Number of carousel slots mounted under <Map> per raster overlay.
 *
 * 1 = today's single-source behaviour (one remount per tick, frame blank until
 * its tiles land). 5 = opacity-swap playback with ≈3 s of hidden prefetch per
 * slot at the 750 ms tick. Changing this changes the native child count under
 * MLRNMapView — flip to 5 only after the on-device checklist in
 * ARCHITECTURE.md § "The app" passes on a physical iPhone.
 */
export const CAROUSEL_WINDOW = 1;

/** Inclusive frame-index range the playback loop cycles through. */
export interface PlaybackWindow {
  start: number;
  end: number;
}

export interface SlotAssignment {
  /** slot index → frame index into the frames array */
  slots: number[];
  /** which slot should be visible (holds the current frame) */
  visibleSlot: number;
}

export function assignSlots(
  currentIndex: number,
  windowStart: number,
  windowEnd: number,
  slotCount: number,
): SlotAssignment {
  // Widen the window if the current frame sits outside it (transient state
  // while the timeline snaps after a zoom/window change) so the visible
  // frame is always assigned to a slot.
  const start = Math.min(windowStart, currentIndex);
  const end = Math.max(windowEnd, currentIndex);
  const winLen = end - start + 1;
  const padded = Math.max(1, Math.ceil(winLen / slotCount)) * slotCount;
  const curP = currentIndex - start;

  const slots = new Array<number>(slotCount);
  for (let k = 0; k < slotCount; k++) {
    const pPadded = (curP + k) % padded;
    const slot = pPadded % slotCount;
    slots[slot] = start + (pPadded % winLen);
  }
  return { slots, visibleSlot: curP % slotCount };
}

/**
 * Clamp a possibly-stale playback window (frames list may have shrunk on a
 * manifest refresh) to valid frame indices. Falls back to the full range.
 */
export function clampWindow(
  window: PlaybackWindow | null,
  frameCount: number,
): PlaybackWindow {
  const last = frameCount - 1;
  if (!window) return { start: 0, end: last };
  const end = Math.max(0, Math.min(window.end, last));
  const start = Math.max(0, Math.min(window.start, end));
  return { start, end };
}
