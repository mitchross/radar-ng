import * as Haptics from "expo-haptics";

/** A light tap for a primary action (play, allow, notify). Silent where haptics are unavailable. */
export function tapHaptic(): void {
  Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(() => {});
}

/** A tick for picking an option (segment, layer, map style). */
export function selectionHaptic(): void {
  Haptics.selectionAsync().catch(() => {});
}
