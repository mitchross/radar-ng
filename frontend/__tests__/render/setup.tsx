// Native modules the render tests never exercise, replaced with in-memory or
// inert stand-ins. Components under test still render their real JSX.
jest.mock("react-native-mmkv", () => {
  const createMMKV = () => {
    const values = new Map<string, string>();
    return {
      getString: (key: string) => values.get(key),
      set: (key: string, value: string) => void values.set(key, value),
      remove: (key: string) => values.delete(key),
      clearAll: () => values.clear(),
    };
  };
  return { createMMKV };
});

jest.mock("react-native-worklets", () => require("react-native-worklets/src/mock"));
jest.mock("react-native-reanimated", () => require("react-native-reanimated/mock"));

jest.mock("expo-router", () => ({ ...jest.requireActual("expo-router"), useIsFocused: () => true }));

jest.mock("react-native-safe-area-context", () => require("react-native-safe-area-context/jest/mock").default);

// Liquid Glass is native-only; render tests exercise the fallback surface.
jest.mock("expo-glass-effect", () => ({
  isLiquidGlassAvailable: () => false,
  GlassView: jest.requireActual("react-native").View,
}));

// Symbols are native views; render nothing in tests (labels live on the buttons).
jest.mock("expo-symbols", () => ({ SymbolView: () => null }));
