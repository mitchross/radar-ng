/**
 * Shared empty collection for map overlays. Overlays always mount their
 * GeoJSONSource with this instead of returning null, so the native child count
 * under MLRNMapView stays constant (iOS NSRangeException in insertReactSubview).
 */
export const EMPTY_FEATURE_COLLECTION: GeoJSON.FeatureCollection = Object.freeze({
  type: "FeatureCollection",
  features: [],
}) as GeoJSON.FeatureCollection;
