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
 * its tiles land). 5 = opacity-swap playback with 2 s of hidden prefetch per
 * slot at the default 500 ms tick. Changing this changes the native child
 * count under MLRNMapView — flip the default to 5 only after the on-device
 * checklist in ARCHITECTURE.md § "The app" passes on a physical iPhone.
 *
 * A build can set EXPO_PUBLIC_CAROUSEL_WINDOW=5 for device testing. The value
 * is inlined at build time, so it never changes while the app runs.
 */
export function parseCarouselWindow(value: string | undefined): 1 | 5 {
  return value?.trim() === "5" ? 5 : 1;
}

export const CAROUSEL_WINDOW = parseCarouselWindow(process.env.EXPO_PUBLIC_CAROUSEL_WINDOW);

/** Inclusive frame-index range the playback loop cycles through. */
export interface PlaybackWindow {
  start: number;
  end: number;
  /**
   * The frame indices playback actually visits, in order, when it skips
   * frames (the 48h view thins the past). Absent means every index.
   */
  sequence?: readonly number[];
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
 * assignSlots along an explicit playback sequence: slot k holds the frame k
 * steps ahead in the sequence, so prefetch follows what playback will show
 * and a one-step advance still changes exactly one slot. Falls back to
 * contiguous assignment when the current frame isn't in the sequence (a
 * scrub to a skipped frame).
 */
export function assignSlotsInSequence(
  currentIndex: number,
  sequence: readonly number[],
  slotCount: number,
): SlotAssignment | null {
  const pos = sequence.indexOf(currentIndex);
  if (pos < 0 || sequence.length === 0) return null;
  const len = sequence.length;
  const padded = Math.max(1, Math.ceil(len / slotCount)) * slotCount;
  const slots = new Array<number>(slotCount);
  for (let k = 0; k < slotCount; k++) {
    const pPadded = (pos + k) % padded;
    slots[pPadded % slotCount] = sequence[pPadded % len];
  }
  return { slots, visibleSlot: pos % slotCount };
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
  const sequence = window.sequence?.filter((i) => i >= 0 && i <= last);
  return sequence && sequence.length > 0 ? { start, end, sequence } : { start, end };
}
