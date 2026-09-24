import { useMemo } from "react";
import { useSafeAreaInsets } from "react-native-safe-area-context";

/** Spacing for chrome over the full-bleed radar map, added to the safe area. */
export const MAP_CHROME = {
  /** Below the close-button row. */
  TOP: 53,
  SIDE: 12,
  BOTTOM: 10,
  /** Height the timeline card takes above its bottom edge. */
  TIMELINE_HEIGHT: 196,
} as const;

/**
 * Edges for radar chrome. On a notched iPhone in portrait these match the
 * old fixed offsets (top 112, timeline bottom 44); elsewhere they follow the
 * status bar, navigation bar and display cutouts.
 */
export function useMapChromeInsets() {
  const insets = useSafeAreaInsets();
  return useMemo(() => {
    const bottom = insets.bottom + MAP_CHROME.BOTTOM;
    return {
      top: insets.top + MAP_CHROME.TOP,
      bottom,
      left: insets.left + MAP_CHROME.SIDE,
      right: insets.right + MAP_CHROME.SIDE,
      aboveTimeline: bottom + MAP_CHROME.TIMELINE_HEIGHT,
    };
  }, [insets.top, insets.bottom, insets.left, insets.right]);
}
