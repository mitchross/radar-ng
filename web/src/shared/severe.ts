/**
 * The "Severe" layers (NOAA MRMS hail size, rotation tracks, echo tops): ids, names, one-line hints,
 * source credit and point readouts, declared once for the web radar and the app.
 */

export const SEVERE_CREDIT = "NOAA MRMS";

export type SevereLayerId = "hail-size" | "rotation-track" | "echo-top";

export interface SevereLayerInfo {
  id: SevereLayerId;
  name: string;
  /** One plain-English line: what the colors mean. */
  hint: string;
  /** Unit the server renders and the inspector reports. */
  unit: string;
}

export const SEVERE_LAYERS: readonly SevereLayerInfo[] = [
  { id: "hail-size", name: "Hail size", hint: "Where hail fell in the last hour, by size", unit: "in" },
  { id: "rotation-track", name: "Rotation tracks", hint: "Paths of strong low-level storm rotation in the last hour", unit: "1/s" },
  { id: "echo-top", name: "Echo tops", hint: "How tall the storms are: taller usually means stronger", unit: "kft" },
];

export function severeLayer(id: string): SevereLayerInfo | undefined {
  return SEVERE_LAYERS.find((l) => l.id === id);
}

export const isSevereLayer = (id: string): boolean => severeLayer(id) !== undefined;

/** Point readout for a severe layer, or null for any other layer. Values are in the layer's unit. */
export function formatSevereReading(layer: string, value: number): string | null {
  switch (layer) {
    case "hail-size":
      return value < 0.25 ? "No hail" : `${value.toFixed(2)} in hail`;
    case "rotation-track":
      return value < 0.003 ? "No rotation" : `${value.toFixed(3)} /s`;
    case "echo-top":
      return value < 5 ? "No storm top" : `${Math.round(value)} kft`;
    default:
      return null;
  }
}
