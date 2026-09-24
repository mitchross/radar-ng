import { requireOptionalNativeModule } from "expo-modules-core";

interface RadarSharedStateNative {
  publish(json: string): void;
}

// iOS only; absent on Android and in tests, where publishing is a no-op.
const native = requireOptionalNativeModule<RadarSharedStateNative>("RadarSharedState");

export function publishSharedState(json: string): void {
  native?.publish(json);
}
