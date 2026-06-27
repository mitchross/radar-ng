# Weather Clear Native Reconciliation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reconcile the iOS/Android Expo app with the Weather Clear reference, including coherent light/dark/system themes, preserved live behavior, accessible state handling, and verified Android-native screenshots.

**Architecture:** Keep React Query, Zustand, Expo Router, MapLibre, and the existing domain hooks intact. Add a semantic Weather Clear theme layer and small shared UI primitives, then migrate each screen from static `cumulus` colors to theme-derived styles while extracting testable presentation-state functions for loading/error/empty distinctions.

**Tech Stack:** Expo SDK 56 preview, React Native 0.85, React 19, TypeScript, Zustand, React Query, Expo Router, MapLibre, Jest/ts-jest, Android emulator/ADB, ImageMagick.

---

## File Map

### Create

- `frontend/src/theme/weatherClearTheme.ts` — semantic light/dark token definitions and pure appearance resolution.
- `frontend/src/theme/WeatherClearThemeProvider.tsx` — native system-scheme resolution and theme context.
- `frontend/src/components/ui/WeatherClearUI.tsx` — shared section label, segmented control, card, screen header, and screen-state primitives.
- `frontend/src/lib/weatherPresentation.ts` — pure forecast, nowcast, and alert view-state derivation.
- `frontend/__tests__/theme/weatherClearTheme.test.ts` — token and appearance resolution coverage.
- `frontend/__tests__/lib/weatherPresentation.test.ts` — loading/error/empty/available presentation coverage.
- `frontend/__tests__/components/weatherClearContracts.test.ts` — source contracts for tab accessibility, screen states, and 44-point controls.
- `docs/audits/2026-06-27-weather-clear-audit.md` — before/after mismatch matrix and verification record.

### Modify

- `frontend/package.json`, `frontend/bun.lock` — native Newsreader and Spline Sans font packages.
- `frontend/src/stores/useWeatherStore.ts` — persisted Light/Dark/System preference without disturbing the user’s default-location edits.
- `frontend/__tests__/stores/useWeatherStore.test.ts` — appearance persistence tests.
- `frontend/src/app/_layout.tsx` — font loading, theme provider, canvas color, and status-bar style.
- `frontend/src/app/(tabs)/_layout.tsx` — reference-aligned themed tab bar.
- `frontend/src/app/(tabs)/index.tsx` — Home reconciliation.
- `frontend/src/screens/NowcastScreen.tsx` — Nowcast reconciliation and unavailable-state distinction.
- `frontend/src/app/(tabs)/alerts.tsx` — alert loading/error/empty/active states.
- `frontend/src/app/alert/[id].tsx` — themed, accessible alert detail.
- `frontend/src/app/(tabs)/settings.tsx` — appearance selector and reference hierarchy.
- `frontend/src/app/(tabs)/radar.tsx` — reference-aligned close affordance and themed chrome.
- `frontend/src/components/home/RadarMiniMap.tsx`, `frontend/src/components/home/StatWidgets.tsx` — themed Home support components.
- `frontend/src/components/inspector/Eyedropper.tsx` — themed inspector surface.
- `frontend/src/components/layers/LayerPicker.tsx` — themed layer picker.
- `frontend/src/components/map/LayerLegendCard.tsx`, `frontend/src/components/map/LayerLocationMarker.tsx`, `frontend/src/components/map/MapStylePicker.tsx`, `frontend/src/components/map/RadarFABs.tsx`, `frontend/src/components/map/WeatherMap.tsx` — themed radar chrome and accessible controls.
- `frontend/src/components/palette/PaletteSelector.tsx` — themed palette selection.
- `frontend/src/components/timeline/CurrentForecastToggle.tsx`, `frontend/src/components/timeline/PlayButton.tsx`, `frontend/src/components/timeline/TimeSlider.tsx`, `frontend/src/components/timeline/TimelineBar.tsx` — themed timeline and control semantics.
- `frontend/src/lib/cumulusTheme.ts` — retain weather-domain helpers while removing responsibility for screen surface colors.
- `frontend/screenshots/01-home.png` through `05-settings.png` — verified post-reconciliation Android captures.

## Task 1: Establish the Audit Baseline

**Files:**

- Create: `docs/audits/2026-06-27-weather-clear-audit.md`
- Read: `Weather app redesign/Weather Clear.dc.html`
- Read: `Weather app redesign/uploads/01-home.png` through `05-settings.png` as historical inputs
- Read: `frontend/screenshots/01-home.png` through `05-settings.png` as duplicate historical inputs

- [ ] **Step 1: Record the clean automated baseline**

Run:

```bash
cd frontend
bun run test -- --runInBand
bunx tsc --noEmit
bun run lint
```

Expected: Jest reports 6 suites and 26 tests passing; lint exits 0; TypeScript reports `StatWidgets.tsx(73,41): TS2353` because the transform contains unsupported `px`.

- [ ] **Step 2: Capture the connected 1080 × 2400 emulator state**

Run:

```bash
adb devices
adb shell wm size
adb shell wm density
adb shell am force-stop com.vanillax.radarng
adb shell monkey -p com.vanillax.radarng -c android.intent.category.LAUNCHER 1
adb shell uiautomator dump /sdcard/radar-ng-home.xml
adb pull /sdcard/radar-ng-home.xml /tmp/radar-ng-home.xml
adb exec-out screencap -p > /tmp/radar-ng-before-home.png
```

Expected: one connected emulator, physical size `1080x2400`, app foregrounded, UI hierarchy saved, and a valid PNG at `/tmp/radar-ng-before-home.png`.

- [ ] **Step 3: Prove the checked-in screenshots are historical duplicates**

Run:

```bash
mkdir -p /tmp/weather-clear-audit
sha256sum "Weather app redesign/uploads/"0{1-home,2-nowcast,3-radar,4-alerts,5-settings}.png
sha256sum frontend/screenshots/0{1-home,2-nowcast,3-radar,4-alerts,5-settings}.png
magick compare -metric AE "Weather app redesign/uploads/01-home.png" \
  "frontend/screenshots/01-home.png" "/tmp/weather-clear-audit/historical-home-diff.png" \
  2>"/tmp/weather-clear-audit/historical-home-ae.txt" || true
```

Expected: every upload hash exactly matches the corresponding frontend screenshot hash and the absolute error is `0`. Record that these are historical dark-app inputs, not targets or current runtime captures.

- [ ] **Step 4: Render the actual Weather Clear target and compare it with native runtime**

Run:

```bash
agent-browser --session weather-clear-reference open \
  "file:///home/vanillax/programming/radar-ng/Weather%20app%20redesign/Weather%20Clear.dc.html"
agent-browser --session weather-clear-reference set viewport 444 892
agent-browser --session weather-clear-reference wait 1000
agent-browser --session weather-clear-reference screenshot /tmp/weather-clear-audit/reference-home.png
adb exec-out screencap -p > /tmp/weather-clear-audit/native-home.png
magick /tmp/weather-clear-audit/reference-home.png \
  /tmp/weather-clear-audit/native-home.png \
  +append /tmp/weather-clear-audit/home-side-by-side.png
```

Expected: the HTML renders the warm paper/Newsreader Weather Clear target. The native capture shows the current Gemini rewrite at its actual current scroll/state. The side-by-side image is qualitative evidence; it is not a pixel metric because the reference is a 384 × 832 design frame and native is 1080 × 2400.

- [ ] **Step 5: Write the concrete baseline audit**

Create `docs/audits/2026-06-27-weather-clear-audit.md` with this content:

```markdown
# Weather Clear Reconciliation Audit

## Baseline

- Device: Android emulator, 1080 × 2400, 420 dpi
- Jest: 6 suites, 26 tests passing
- Lint: passing
- TypeScript: failing at `StatWidgets.tsx:73` because `{ perspective: { px: number } }` is not a valid React Native transform
- Reference: `Weather app redesign/Weather Clear.dc.html`
- Historical evidence: `Weather app redesign/uploads/*` and `frontend/screenshots/*` are matching copies of the old dark UI and are not target captures

## Screen findings

| Screen | Structure | Type/spacing | Color/surfaces | Interaction/state | Accessibility |
| --- | --- | --- | --- | --- | --- |
| Home | Current runtime broadly follows the light redesign but needs top-state capture and hierarchy reconciliation | System serif/sans fallbacks differ from Newsreader/Spline Sans | Warm light palette exists but is hard-coded | Live data and scrolling work; stale/error distinctions require audit | Location and card summaries need explicit labels |
| Nowcast | Must be checked against the HTML Simple and Advanced states | Static screen styles can drift independently | Hard-coded light-only surfaces | Missing minutely data is currently presented as loading or dry | Chart needs a textual summary |
| Radar | Full-screen structure is retained | Chrome sizing differs from the 44-point target | Map chrome assumes dark glass | Map data/gesture behavior must be preserved | Several icon-only controls require labels/state |
| Alerts | Empty-state structure exists | Typography uses platform fallbacks | Static light-only card surfaces | Query failure can read as “All clear” | Severity text exists; refresh target needs audit |
| Settings | Functional groups exceed the static reference and must remain | Large single file has repeated row styles | Static light-only surfaces | Appearance preference is absent and map style must remain independent | Inputs, sliders, and icon controls need complete labels |

## Root causes

1. Screen colors are static module constants, so native system appearance cannot propagate.
2. Native font fallbacks do not match the Newsreader/Spline Sans reference.
3. Repeated screen chrome has drifted because it is implemented independently.
4. Several data screens collapse unavailable data into loading or empty states.
5. Checked-in screenshots were copied from the design inputs, so they cannot validate the current native build.
```

- [ ] **Step 6: Commit the audit baseline**

```bash
git add docs/audits/2026-06-27-weather-clear-audit.md
git commit -m "docs: audit Weather Clear implementation gaps"
```

## Task 2: Add the Semantic Theme and Persisted Appearance

**Files:**

- Create: `frontend/src/theme/weatherClearTheme.ts`
- Create: `frontend/src/theme/WeatherClearThemeProvider.tsx`
- Create: `frontend/__tests__/theme/weatherClearTheme.test.ts`
- Modify: `frontend/src/stores/useWeatherStore.ts`
- Modify: `frontend/__tests__/stores/useWeatherStore.test.ts`

- [ ] **Step 1: Write failing token-resolution tests**

Create `frontend/__tests__/theme/weatherClearTheme.test.ts`:

```ts
import {
  DARK_WEATHER_CLEAR_THEME,
  LIGHT_WEATHER_CLEAR_THEME,
  resolveAppearance,
  selectWeatherClearTheme,
} from "../../src/theme/weatherClearTheme";

describe("Weather Clear appearance", () => {
  it.each([
    ["light", "dark", "light"],
    ["dark", "light", "dark"],
    ["system", "dark", "dark"],
    ["system", "light", "light"],
    ["system", null, "light"],
  ] as const)("resolves %s with native %s to %s", (preference, native, expected) => {
    expect(resolveAppearance(preference, native)).toBe(expected);
  });

  it("uses the canonical paper and ink colors in light mode", () => {
    expect(LIGHT_WEATHER_CLEAR_THEME.colors.canvas).toBe("#f6f2ea");
    expect(LIGHT_WEATHER_CLEAR_THEME.colors.text).toBe("#211f1b");
    expect(LIGHT_WEATHER_CLEAR_THEME.colors.accent).toBe("#c2603a");
  });

  it("uses readable dark surfaces without changing semantic status colors", () => {
    expect(DARK_WEATHER_CLEAR_THEME.dark).toBe(true);
    expect(DARK_WEATHER_CLEAR_THEME.colors.canvas).toBe("#14130f");
    expect(DARK_WEATHER_CLEAR_THEME.colors.text).toBe("#f5efe4");
    expect(selectWeatherClearTheme("dark").colors.success).toBe("#56b97a");
  });
});
```

- [ ] **Step 2: Write failing appearance-store tests**

Append to `frontend/__tests__/stores/useWeatherStore.test.ts`:

```ts
it("starts in system appearance mode", () => {
  expect(useWeatherStore.getState().appearanceMode).toBe("system");
});

it("persists an explicit appearance mode independently of map style", () => {
  useWeatherStore.getState().setMapStyle("satellite");
  useWeatherStore.getState().setAppearanceMode("dark");

  expect(useWeatherStore.getState().appearanceMode).toBe("dark");
  expect(useWeatherStore.getState().mapStyle).toBe("satellite");
  expect(setString).toHaveBeenCalledWith("appearanceMode", "dark");
});
```

- [ ] **Step 3: Run the tests and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/theme/weatherClearTheme.test.ts __tests__/stores/useWeatherStore.test.ts --runInBand
```

Expected: FAIL because the theme module and appearance store fields do not exist.

- [ ] **Step 4: Implement the semantic tokens**

Create `frontend/src/theme/weatherClearTheme.ts` with these public contracts:

```ts
import type { ColorSchemeName } from "react-native";

export type AppearanceMode = "light" | "dark" | "system";
export type ResolvedAppearance = "light" | "dark";

export interface WeatherClearTheme {
  dark: boolean;
  colors: {
    canvas: string;
    surface: string;
    surfaceStrong: string;
    surfaceMuted: string;
    border: string;
    divider: string;
    text: string;
    textSecondary: string;
    textMuted: string;
    textFaint: string;
    accent: string;
    accentSoft: string;
    accentBorder: string;
    success: string;
    warning: string;
    destructive: string;
    rain: string;
    cold: string;
    hot: string;
    scrim: string;
  };
  typography: {
    display: string;
    displayItalic: string;
    ui: string;
    uiMedium: string;
    uiSemibold: string;
    uiBold: string;
    mono: string;
  };
  spacing: { xs: 4; sm: 8; md: 12; lg: 16; xl: 24; xxl: 32 };
  radii: { sm: 8; md: 12; lg: 18; xl: 22; pill: 999 };
  controlMinSize: 44;
}

const shared = {
  spacing: { xs: 4, sm: 8, md: 12, lg: 16, xl: 24, xxl: 32 },
  radii: { sm: 8, md: 12, lg: 18, xl: 22, pill: 999 },
  controlMinSize: 44,
  typography: {
    display: "Newsreader_400Regular",
    displayItalic: "Newsreader_400Regular_Italic",
    ui: "SplineSans_400Regular",
    uiMedium: "SplineSans_500Medium",
    uiSemibold: "SplineSans_600SemiBold",
    uiBold: "SplineSans_700Bold",
    mono: "monospace",
  },
} as const;

export const LIGHT_WEATHER_CLEAR_THEME: WeatherClearTheme = {
  ...shared,
  dark: false,
  colors: {
    canvas: "#f6f2ea",
    surface: "#ffffff",
    surfaceStrong: "#fbf9f5",
    surfaceMuted: "#eae4d8",
    border: "#eee6d8",
    divider: "#e7e0d3",
    text: "#211f1b",
    textSecondary: "#5d574d",
    textMuted: "#8c857a",
    textFaint: "#a39a8a",
    accent: "#c2603a",
    accentSoft: "rgba(194,96,58,0.12)",
    accentBorder: "rgba(194,96,58,0.30)",
    success: "#56b97a",
    warning: "#f0c34e",
    destructive: "#df6a6a",
    rain: "#4d7fb8",
    cold: "#6db4d8",
    hot: "#df6a3c",
    scrim: "rgba(33,31,27,0.45)",
  },
};

export const DARK_WEATHER_CLEAR_THEME: WeatherClearTheme = {
  ...shared,
  dark: true,
  colors: {
    canvas: "#14130f",
    surface: "#1d1b16",
    surfaceStrong: "#242119",
    surfaceMuted: "#302c24",
    border: "#3a352b",
    divider: "#332f27",
    text: "#f5efe4",
    textSecondary: "#c9bdac",
    textMuted: "#a79b89",
    textFaint: "#81786b",
    accent: "#d9825d",
    accentSoft: "rgba(217,130,93,0.16)",
    accentBorder: "rgba(217,130,93,0.36)",
    success: "#56b97a",
    warning: "#e8bd55",
    destructive: "#e47d7d",
    rain: "#76a2d4",
    cold: "#79bddb",
    hot: "#e47a50",
    scrim: "rgba(0,0,0,0.62)",
  },
};

export function resolveAppearance(
  preference: AppearanceMode,
  nativeScheme: ColorSchemeName,
): ResolvedAppearance {
  if (preference !== "system") return preference;
  return nativeScheme === "dark" ? "dark" : "light";
}

export function selectWeatherClearTheme(
  resolved: ResolvedAppearance,
): WeatherClearTheme {
  return resolved === "dark"
    ? DARK_WEATHER_CLEAR_THEME
    : LIGHT_WEATHER_CLEAR_THEME;
}
```

- [ ] **Step 5: Implement persisted appearance without overwriting the user’s location hunks**

In `frontend/src/stores/useWeatherStore.ts`, add:

```ts
import type { AppearanceMode } from "../theme/weatherClearTheme";

function parseAppearanceMode(value: string): AppearanceMode {
  return value === "light" || value === "dark" ? value : "system";
}
```

Add `appearanceMode: AppearanceMode` and `setAppearanceMode: (mode: AppearanceMode) => void` to `WeatherState`, then add:

```ts
appearanceMode: parseAppearanceMode(getString("appearanceMode", "system")),
setAppearanceMode: (mode) => {
  setString("appearanceMode", mode);
  set({ appearanceMode: mode });
},
```

Do not stage or rewrite the existing `DEFAULT_PLACE`, `initialSelectedPlace`, latitude, or longitude hunks as part of this task.

- [ ] **Step 6: Add the theme provider**

Create `frontend/src/theme/WeatherClearThemeProvider.tsx`:

```tsx
import { createContext, useContext, useMemo, type ReactNode } from "react";
import { useColorScheme } from "react-native";
import { useWeatherStore } from "../stores/useWeatherStore";
import {
  resolveAppearance,
  selectWeatherClearTheme,
  type ResolvedAppearance,
  type WeatherClearTheme,
} from "./weatherClearTheme";

type ThemeContextValue = {
  theme: WeatherClearTheme;
  resolvedAppearance: ResolvedAppearance;
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function WeatherClearThemeProvider({ children }: { children: ReactNode }) {
  const preference = useWeatherStore((state) => state.appearanceMode);
  const nativeScheme = useColorScheme();
  const resolvedAppearance = resolveAppearance(preference, nativeScheme);
  const value = useMemo(
    () => ({
      resolvedAppearance,
      theme: selectWeatherClearTheme(resolvedAppearance),
    }),
    [resolvedAppearance],
  );
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useWeatherClearTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) throw new Error("useWeatherClearTheme requires WeatherClearThemeProvider");
  return value;
}
```

- [ ] **Step 7: Run the focused tests and verify GREEN**

Run:

```bash
cd frontend
bun run test -- __tests__/theme/weatherClearTheme.test.ts __tests__/stores/useWeatherStore.test.ts --runInBand
```

Expected: both suites PASS.

- [ ] **Step 8: Commit only the theme/store changes**

Use interactive staging for the store because it contains pre-existing user changes:

```bash
git add frontend/src/theme frontend/__tests__/theme frontend/__tests__/stores/useWeatherStore.test.ts
git add -p frontend/src/stores/useWeatherStore.ts
git diff --cached --check
git commit -m "feat: add Weather Clear appearance themes"
```

## Task 3: Load Native Fonts and Build Shared Chrome

**Files:**

- Modify: `frontend/package.json`
- Modify: `frontend/bun.lock`
- Modify: `frontend/src/app/_layout.tsx`
- Create: `frontend/src/components/ui/WeatherClearUI.tsx`
- Modify: `frontend/src/app/(tabs)/_layout.tsx`
- Create: `frontend/__tests__/components/weatherClearContracts.test.ts`

- [ ] **Step 1: Install the two reference font families**

Run:

```bash
cd frontend
bun add @expo-google-fonts/newsreader @expo-google-fonts/spline-sans
```

Expected: both dependencies are recorded in `package.json` and `bun.lock`.

- [ ] **Step 2: Write failing chrome contracts**

Create `frontend/__tests__/components/weatherClearContracts.test.ts`:

```ts
import { readFileSync } from "fs";
import path from "path";

function source(relativePath: string): string {
  return readFileSync(path.join(__dirname, "../../src", relativePath), "utf8");
}

describe("Weather Clear native UI contracts", () => {
  it("loads both design font families before rendering navigation", () => {
    const root = source("app/_layout.tsx");
    expect(root).toContain("Newsreader_400Regular");
    expect(root).toContain("SplineSans_400Regular");
    expect(root).toContain("if (!fontsLoaded)");
  });

  it("keeps all tab controls accessible and at least 44 points tall", () => {
    const tabs = source("app/(tabs)/_layout.tsx");
    expect(tabs).toContain('accessibilityRole="tab"');
    expect(tabs).toContain("accessibilityState={{ selected: active }}");
    expect(tabs).toContain("minHeight: 44");
  });

  it("hides the tab bar only on the full-screen radar route", () => {
    const tabs = source("app/(tabs)/_layout.tsx");
    expect(tabs).toContain('activeRoute === "radar"');
  });
});
```

- [ ] **Step 3: Run the contract test and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/components/weatherClearContracts.test.ts --runInBand
```

Expected: FAIL because the font-loading and minimum tab size contracts are absent.

- [ ] **Step 4: Implement shared UI primitives**

Create `frontend/src/components/ui/WeatherClearUI.tsx` exporting:

```tsx
export function WeatherClearCard(props: ViewProps & { children: ReactNode }): JSX.Element;
export function SectionLabel(props: { children: string; trailing?: string }): JSX.Element;
export function ScreenHeader(props: {
  kicker?: string;
  title: string;
  action?: ReactNode;
}): JSX.Element;
export function SegmentedControl<T extends string>(props: {
  accessibilityLabel: string;
  options: readonly { label: string; value: T }[];
  value: T;
  onChange: (value: T) => void;
}): JSX.Element;
export function ScreenState(props: {
  kind: "loading" | "error" | "empty";
  title: string;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}): JSX.Element;
```

Each pressable must use `minWidth: theme.controlMinSize`, `minHeight: theme.controlMinSize`, an accessibility role/label/state, and theme-derived colors. `ScreenState` must use `accessibilityRole="alert"` only for `kind="error"` and must not animate when the system requests reduced motion.

- [ ] **Step 5: Load fonts and apply the root theme**

In `frontend/src/app/_layout.tsx`:

```tsx
import {
  Newsreader_400Regular,
  Newsreader_400Regular_Italic,
  Newsreader_500Medium,
} from "@expo-google-fonts/newsreader";
import {
  SplineSans_400Regular,
  SplineSans_500Medium,
  SplineSans_600SemiBold,
  SplineSans_700Bold,
} from "@expo-google-fonts/spline-sans";
import { useFonts } from "expo-font";
import { WeatherClearThemeProvider, useWeatherClearTheme } from "../theme/WeatherClearThemeProvider";
```

Load all seven faces with one `useFonts` call, return `null` until it reports loaded, wrap the navigation tree with `WeatherClearThemeProvider`, and render the status bar from a themed child:

```tsx
function ThemedStatusBar() {
  const { resolvedAppearance } = useWeatherClearTheme();
  return <StatusBar style={resolvedAppearance === "dark" ? "light" : "dark"} />;
}
```

- [ ] **Step 6: Rebuild the tab bar against the reference**

In `frontend/src/app/(tabs)/_layout.tsx`:

- use `useWeatherClearTheme()`;
- use a single top separator rather than a floating dark pill;
- use themed canvas, divider, muted, and accent colors;
- retain the five labels and alert badge;
- use `paddingBottom` from safe-area insets;
- set every tab item to `minHeight: 44`;
- keep `activeRoute === "radar"` as the only hidden case.

- [ ] **Step 7: Verify focused tests and type checking**

Run:

```bash
cd frontend
bun run test -- __tests__/components/weatherClearContracts.test.ts --runInBand
bunx tsc --noEmit
```

Expected: contract test PASS and TypeScript exits 0.

- [ ] **Step 8: Commit shared chrome**

```bash
git add frontend/package.json frontend/bun.lock frontend/src/app/_layout.tsx \
  'frontend/src/app/(tabs)/_layout.tsx' frontend/src/components/ui \
  frontend/__tests__/components/weatherClearContracts.test.ts
git diff --cached --check
git commit -m "feat: add Weather Clear native UI foundation"
```

## Task 4: Reconcile Home and Its Presentation Logic

**Files:**

- Create: `frontend/src/lib/weatherPresentation.ts`
- Create: `frontend/__tests__/lib/weatherPresentation.test.ts`
- Modify: `frontend/src/app/(tabs)/index.tsx`
- Modify: `frontend/src/components/home/RadarMiniMap.tsx`
- Modify: `frontend/src/components/home/StatWidgets.tsx`
- Modify: `frontend/src/lib/cumulusTheme.ts`

- [ ] **Step 1: Write failing Home presentation tests**

Create `frontend/__tests__/lib/weatherPresentation.test.ts` with:

```ts
import { getForecastScreenState, getNowcastVerdict } from "../../src/lib/weatherPresentation";

describe("forecast presentation", () => {
  it("does not label unavailable forecast data as loading after a request fails", () => {
    expect(getForecastScreenState({
      data: undefined,
      isLoading: false,
      isError: true,
      isFetching: false,
    })).toEqual({ kind: "error", stale: false });
  });

  it("keeps stale forecast data visible while refresh fails", () => {
    expect(getForecastScreenState({
      data: { current: {} },
      isLoading: false,
      isError: true,
      isFetching: false,
    })).toEqual({ kind: "content", stale: true });
  });

  it("distinguishes dry nowcast from unavailable minute data", () => {
    expect(getNowcastVerdict(undefined)).toEqual({ kind: "unavailable" });
    expect(getNowcastVerdict([0, 0, 0, 0])).toEqual({ kind: "dry" });
  });
});
```

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/lib/weatherPresentation.test.ts --runInBand
```

Expected: FAIL because `weatherPresentation.ts` does not exist.

- [ ] **Step 3: Implement pure presentation states**

Create `frontend/src/lib/weatherPresentation.ts`:

```ts
export type QueryPresentation = {
  kind: "loading" | "error" | "content";
  stale: boolean;
};

export function getForecastScreenState(input: {
  data: unknown;
  isLoading: boolean;
  isError: boolean;
  isFetching: boolean;
}): QueryPresentation {
  if (input.data) return { kind: "content", stale: input.isError };
  if (input.isLoading || input.isFetching) return { kind: "loading", stale: false };
  return { kind: "error", stale: false };
}

export type NowcastVerdict =
  | { kind: "unavailable" }
  | { kind: "dry" }
  | { kind: "raining"; peakMinute: number; endMinute: number }
  | { kind: "starting"; startMinute: number; peakMinute: number; endMinute: number };

export function getNowcastVerdict(values: number[] | undefined): NowcastVerdict {
  if (!values?.length) return { kind: "unavailable" };
  const startMinute = values.findIndex((value) => value > 0.08);
  if (startMinute < 0) return { kind: "dry" };
  const peakMinute = values.reduce(
    (best, value, index) => value > values[best] ? index : best,
    0,
  );
  let endMinute = values.length - 1;
  while (endMinute > startMinute && values[endMinute] <= 0.05) endMinute -= 1;
  return startMinute === 0
    ? { kind: "raining", peakMinute, endMinute }
    : { kind: "starting", startMinute, peakMinute, endMinute };
}
```

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cd frontend
bun run test -- __tests__/lib/weatherPresentation.test.ts --runInBand
```

Expected: PASS.

- [ ] **Step 5: Reconcile Home hierarchy and themed styles**

In `frontend/src/app/(tabs)/index.tsx`:

- use `getForecastScreenState()` before rendering;
- render `ScreenState` for loading and hard error while leaving stale content visible;
- use `useWeatherClearTheme()` and `createStyles(theme)` rather than static surface colors;
- use the shared `SegmentedControl` and `SectionLabel`;
- match the reference order: location/mode, condition/temperature, hourly, precipitation, seven-day, then Advanced metrics;
- use Newsreader for condition, temperature, forecast values, and section totals;
- use Spline Sans for labels and controls;
- preserve refresh, live data, location navigation, alert banner, and Simple/Advanced behavior;
- add accessibility labels to the location selector, hourly cards, forecast rows, and precipitation summary;
- keep bottom content padding large enough for the themed tab bar.

In `RadarMiniMap.tsx` and `StatWidgets.tsx`, accept theme colors through `useWeatherClearTheme()` and remove static white/paper assumptions. Keep weather/radar semantic colors unchanged.

Replace the invalid `[{ perspective: { px: 800 } }]` transform in `StatWidgets.tsx` with the React Native form `[{ perspective: 800 }]`; this resolves the verified baseline TypeScript failure without changing the intended perspective.

In `cumulusTheme.ts`, retain `getCumulusCondition`, `getIconKind`, `getUVInfo`, `getWindInfo`, `getWindDirection`, `isNightAt`, `DBZ_SCALE`, and condition labels. Stop using it as the source of app canvas/card/text colors.

- [ ] **Step 6: Verify Home behavior and capture a visual checkpoint**

Run:

```bash
cd frontend
bun run test -- __tests__/lib/weatherPresentation.test.ts --runInBand
bunx tsc --noEmit
adb shell am force-stop com.vanillax.radarng
adb shell monkey -p com.vanillax.radarng -c android.intent.category.LAUNCHER 1
adb exec-out screencap -p > /tmp/weather-clear-home-after.png
agent-browser --session weather-clear-reference open \
  "file:///home/vanillax/programming/radar-ng/Weather%20app%20redesign/Weather%20Clear.dc.html"
agent-browser --session weather-clear-reference set viewport 444 892
agent-browser --session weather-clear-reference screenshot /tmp/weather-clear-reference-home.png
magick /tmp/weather-clear-reference-home.png -crop 384x832+30+30 +repage \
  -resize 1080x2400\\! /tmp/weather-clear-reference-home-normalized.png
magick /tmp/weather-clear-reference-home-normalized.png /tmp/weather-clear-home-after.png \
  +append /tmp/weather-clear-audit/01-home-after-side-by-side.png
```

Expected: tests and type checking pass; app launches; capture shows reference hierarchy without clipped controls.

- [ ] **Step 7: Commit Home**

```bash
git add frontend/src/lib/weatherPresentation.ts \
  frontend/__tests__/lib/weatherPresentation.test.ts \
  'frontend/src/app/(tabs)/index.tsx' \
  frontend/src/components/home/RadarMiniMap.tsx \
  frontend/src/components/home/StatWidgets.tsx \
  frontend/src/lib/cumulusTheme.ts
git diff --cached --check
git commit -m "feat: reconcile Weather Clear home"
```

## Task 5: Reconcile Nowcast and Its Data States

**Files:**

- Modify: `frontend/__tests__/lib/weatherPresentation.test.ts`
- Modify: `frontend/src/lib/weatherPresentation.ts`
- Modify: `frontend/src/screens/NowcastScreen.tsx`

- [ ] **Step 1: Add a failing accessible-summary test**

Extend the existing `weatherPresentation` import to include `describeNowcast`, then append:

```ts
it("summarizes precipitation timing without relying on the chart", () => {
  expect(describeNowcast({
    kind: "starting",
    startMinute: 15,
    peakMinute: 30,
    endMinute: 50,
  })).toBe("Rain starts in 15 minutes, peaks at 30 minutes, and ends near 50 minutes.");
});
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/lib/weatherPresentation.test.ts --runInBand
```

Expected: FAIL because `describeNowcast` is not exported.

- [ ] **Step 3: Implement the minimal textual summary**

Add `describeNowcast(verdict: NowcastVerdict): string` with exact text for unavailable, dry, raining, and starting states. Use that function for the chart accessibility label so visual and assistive output share one source.

- [ ] **Step 4: Reconcile Nowcast**

In `frontend/src/screens/NowcastScreen.tsx`:

- use `getForecastScreenState()` and `getNowcastVerdict()`;
- render “Forecast unavailable” when minutely data is absent, never “No rain expected”;
- match the reference header, two-line verdict, intensity card, key-moment grid, and Advanced model table;
- use the shared segmented control and screen-state primitives;
- use themed dynamic styles and native fonts;
- preserve refresh and location behavior;
- expose chart meaning through an accessibility summary such as `Next hour precipitation: dry` or `Rain starts in 15 minutes, peaks at 30 minutes`.

- [ ] **Step 5: Verify and capture**

Run:

```bash
cd frontend
bun run test -- __tests__/lib/weatherPresentation.test.ts --runInBand
bunx tsc --noEmit
adb shell input tap 324 2260
adb exec-out screencap -p > /tmp/weather-clear-nowcast-after.png
```

Expected: tests pass, TypeScript exits 0, Nowcast opens, and the screenshot contains the reference hierarchy.

- [ ] **Step 6: Commit Nowcast**

```bash
git add frontend/src/screens/NowcastScreen.tsx \
  frontend/src/lib/weatherPresentation.ts \
  frontend/__tests__/lib/weatherPresentation.test.ts
git diff --cached --check
git commit -m "feat: reconcile Weather Clear nowcast"
```

## Task 6: Reconcile Alerts and Alert Detail

**Files:**

- Modify: `frontend/src/lib/weatherPresentation.ts`
- Modify: `frontend/__tests__/lib/weatherPresentation.test.ts`
- Modify: `frontend/src/app/(tabs)/alerts.tsx`
- Modify: `frontend/src/app/alert/[id].tsx`

- [ ] **Step 1: Write failing alert-state tests**

Extend the existing `weatherPresentation` import to include `getAlertsScreenState`, then append:

```ts
it.each([
  [{ data: undefined, isLoading: true, isError: false }, "loading"],
  [{ data: undefined, isLoading: false, isError: true }, "error"],
  [{ data: { features: [] }, isLoading: false, isError: false }, "empty"],
  [{ data: { features: [{ id: "one" }] }, isLoading: false, isError: false }, "content"],
] as const)("maps alert query state to %s", (input, kind) => {
  expect(getAlertsScreenState(input).kind).toBe(kind);
});
```

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/lib/weatherPresentation.test.ts --runInBand
```

Expected: FAIL because `getAlertsScreenState` is not exported.

- [ ] **Step 3: Implement alert-state derivation**

Add:

```ts
export function getAlertsScreenState(input: {
  data: { features: unknown[] } | undefined;
  isLoading: boolean;
  isError: boolean;
}): { kind: "loading" | "error" | "empty" | "content" } {
  if (input.data?.features.length) return { kind: "content" };
  if (input.data) return { kind: "empty" };
  if (input.isLoading) return { kind: "loading" };
  return { kind: input.isError ? "error" : "loading" };
}
```

- [ ] **Step 4: Reconcile Alerts**

In `frontend/src/app/(tabs)/alerts.tsx`:

- consume `isError` and `getAlertsScreenState`;
- match the reference “All clear” header and centered success state;
- add a real error state with retry rather than rendering “All clear” on failure;
- theme active alert cards and retain textual severity/urgency labels so color is supplemental;
- give refresh a 44-point target and a descriptive label;
- preserve Simple/Advanced metadata and alert navigation.

- [ ] **Step 5: Reconcile alert detail**

In `frontend/src/app/alert/[id].tsx`:

- migrate static colors to `useWeatherClearTheme()` and `createStyles(theme)`;
- keep severity-specific accent colors;
- provide accessible close/back, map, time, area, instruction, and source sections;
- keep the full NWS text and polygon behavior;
- show an explicit unavailable state when the route id is missing from cached alerts.

- [ ] **Step 6: Verify**

Run:

```bash
cd frontend
bun run test -- __tests__/lib/weatherPresentation.test.ts --runInBand
bunx tsc --noEmit
adb shell input tap 756 2260
adb exec-out screencap -p > /tmp/weather-clear-alerts-after.png
```

Expected: tests pass, Alerts opens, and empty/content/error state logic is no longer conflated.

- [ ] **Step 7: Commit Alerts**

```bash
git add frontend/src/lib/weatherPresentation.ts \
  frontend/__tests__/lib/weatherPresentation.test.ts \
  'frontend/src/app/(tabs)/alerts.tsx' \
  'frontend/src/app/alert/[id].tsx'
git diff --cached --check
git commit -m "feat: reconcile Weather Clear alerts"
```

## Task 7: Reconcile Settings and Add Appearance Controls

**Files:**

- Modify: `frontend/src/app/(tabs)/settings.tsx`
- Modify: `frontend/src/components/palette/PaletteSelector.tsx`
- Modify: `frontend/__tests__/components/weatherClearContracts.test.ts`

- [ ] **Step 1: Add a failing appearance-control contract**

Append:

```ts
it("offers Light, Dark, and System independently of radar map style", () => {
  const settings = source("app/(tabs)/settings.tsx");
  expect(settings).toContain('value: "light"');
  expect(settings).toContain('value: "dark"');
  expect(settings).toContain('value: "system"');
  expect(settings).toContain("setAppearanceMode");
  expect(settings).toContain("setMapStyle");
});
```

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/components/weatherClearContracts.test.ts --runInBand
```

Expected: FAIL because appearance controls are absent.

- [ ] **Step 3: Reconcile Settings**

In `frontend/src/app/(tabs)/settings.tsx`:

- migrate to dynamic themed styles and shared section/card/segmented primitives;
- match the reference title, mode control, grouped rows, and restrained diagnostics hierarchy;
- add an Appearance row using:

```tsx
<SegmentedControl
  accessibilityLabel="App appearance"
  options={[
    { label: "Light", value: "light" },
    { label: "Dark", value: "dark" },
    { label: "System", value: "system" },
  ]}
  value={appearanceMode}
  onChange={setAppearanceMode}
/>
```

- retain the separate Radar map theme/style control;
- preserve city search, GPS selection, units, radar palette/opacity, playback speed, server URL, health status, data sources, and Advanced diagnostics;
- label inputs and controls for assistive technology;
- ensure disabled controls remain readable in both themes.

Update `PaletteSelector.tsx` to use themed borders, text, selected state, and 44-point choices.

- [ ] **Step 4: Verify behavior**

Run:

```bash
cd frontend
bun run test -- __tests__/components/weatherClearContracts.test.ts \
  __tests__/stores/useWeatherStore.test.ts --runInBand
bunx tsc --noEmit
adb shell input tap 972 2260
adb exec-out screencap -p > /tmp/weather-clear-settings-light.png
```

Then select Dark in the running app and run:

```bash
adb exec-out screencap -p > /tmp/weather-clear-settings-dark.png
adb shell am force-stop com.vanillax.radarng
adb shell monkey -p com.vanillax.radarng -c android.intent.category.LAUNCHER 1
adb exec-out screencap -p > /tmp/weather-clear-dark-relaunch.png
```

Expected: settings and store tests pass; dark appearance applies immediately and survives relaunch; radar map style remains unchanged.

- [ ] **Step 5: Commit Settings**

```bash
git add 'frontend/src/app/(tabs)/settings.tsx' \
  frontend/src/components/palette/PaletteSelector.tsx \
  frontend/__tests__/components/weatherClearContracts.test.ts
git diff --cached --check
git commit -m "feat: add Weather Clear appearance settings"
```

## Task 8: Reconcile Radar Chrome Without Changing Map Behavior

**Files:**

- Modify: `frontend/src/app/(tabs)/radar.tsx`
- Modify: `frontend/src/components/inspector/Eyedropper.tsx`
- Modify: `frontend/src/components/layers/LayerPicker.tsx`
- Modify: `frontend/src/components/map/LayerLegendCard.tsx`
- Modify: `frontend/src/components/map/LayerLocationMarker.tsx`
- Modify: `frontend/src/components/map/MapStylePicker.tsx`
- Modify: `frontend/src/components/map/RadarFABs.tsx`
- Modify: `frontend/src/components/map/WeatherMap.tsx`
- Modify: `frontend/src/components/timeline/CurrentForecastToggle.tsx`
- Modify: `frontend/src/components/timeline/PlayButton.tsx`
- Modify: `frontend/src/components/timeline/TimeSlider.tsx`
- Modify: `frontend/src/components/timeline/TimelineBar.tsx`
- Modify: `frontend/__tests__/components/WeatherMap.test.ts`
- Modify: `frontend/__tests__/components/weatherClearContracts.test.ts`

- [ ] **Step 1: Add failing map-preservation and accessibility contracts**

Append to `frontend/__tests__/components/WeatherMap.test.ts`:

```ts
it("keeps zoom controls accessible with minimum native targets", () => {
  const source = readFileSync(
    path.join(__dirname, "../../src/components/map/WeatherMap.tsx"),
    "utf8",
  );
  expect(source).toContain('accessibilityRole="button"');
  expect(source).toContain("minWidth: 44");
  expect(source).toContain("minHeight: 44");
});
```

Append to `weatherClearContracts.test.ts`:

```ts
it("keeps radar close, playback, layers, and map style controls labeled", () => {
  const radar = source("app/(tabs)/radar.tsx");
  const fabs = source("components/map/RadarFABs.tsx");
  const timeline = source("components/timeline/TimelineBar.tsx");
  expect(radar).toContain('accessibilityLabel="Close radar"');
  expect(fabs).toContain("accessibilityLabel");
  expect(timeline).toContain("accessibilityLabel");
});
```

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/components/WeatherMap.test.ts \
  __tests__/components/weatherClearContracts.test.ts --runInBand
```

Expected: at least the 44-point zoom-control contract FAILS.

- [ ] **Step 3: Reconcile radar surfaces and controls**

For each file in this task:

- call `useWeatherClearTheme()` in the rendered component;
- move static screen-surface styles to `createStyles(theme)`;
- preserve radar data colors, basemap styles, raster opacity, layer logic, camera behavior, and MapLibre source/layer order;
- use light frosted surfaces in light appearance and dark translucent surfaces in dark appearance;
- make every floating control at least 44 × 44;
- add label, role, selected/expanded state, and hint where applicable;
- keep the full-screen tab-bar suppression;
- match the reference close button, vertical legend, right rail, and bottom timeline placement;
- keep map controls usable when tiles fail or are still loading.

Do not alter `RadarOverlay`, `WeatherLayerOverlay`, query keys, tile URLs, or frame playback algorithms unless a failing existing test identifies a regression.

- [ ] **Step 4: Verify radar behavior**

Run:

```bash
cd frontend
bun run test -- __tests__/components/WeatherMap.test.ts \
  __tests__/lib/tileUrl.test.ts \
  __tests__/components/weatherClearContracts.test.ts --runInBand
bunx tsc --noEmit
adb shell input tap 540 2260
adb shell uiautomator dump /sdcard/radar-ng-radar.xml
adb pull /sdcard/radar-ng-radar.xml /tmp/radar-ng-radar.xml
adb exec-out screencap -p > /tmp/weather-clear-radar-after.png
```

Expected: tests pass; radar opens full-screen; controls appear in the UI hierarchy; map remains interactive.

- [ ] **Step 5: Commit Radar**

```bash
git add 'frontend/src/app/(tabs)/radar.tsx' \
  frontend/src/components/inspector/Eyedropper.tsx \
  frontend/src/components/layers/LayerPicker.tsx \
  frontend/src/components/map/LayerLegendCard.tsx \
  frontend/src/components/map/LayerLocationMarker.tsx \
  frontend/src/components/map/MapStylePicker.tsx \
  frontend/src/components/map/RadarFABs.tsx \
  frontend/src/components/map/WeatherMap.tsx \
  frontend/src/components/timeline/CurrentForecastToggle.tsx \
  frontend/src/components/timeline/PlayButton.tsx \
  frontend/src/components/timeline/TimeSlider.tsx \
  frontend/src/components/timeline/TimelineBar.tsx \
  frontend/__tests__/components/WeatherMap.test.ts \
  frontend/__tests__/components/weatherClearContracts.test.ts
git diff --cached --check
git commit -m "feat: reconcile Weather Clear radar chrome"
```

## Task 9: Complete the Native Accessibility and Responsive Pass

**Files:**

- Modify: all screen and shared UI files changed in Tasks 3–8
- Modify: `frontend/__tests__/components/weatherClearContracts.test.ts`

- [ ] **Step 1: Add failing cross-screen accessibility contracts**

Add assertions that every main screen source includes an accessibility label for its primary heading or container, every refresh control has an action label, and all icon-only pressables have labels.

Use exact source paths and exact expected label strings:

```ts
it.each([
  ["app/(tabs)/index.tsx", "Current weather"],
  ["screens/NowcastScreen.tsx", "Next hour precipitation"],
  ["app/(tabs)/alerts.tsx", "Weather alerts"],
  ["app/(tabs)/settings.tsx", "Weather settings"],
] as const)("%s exposes %s", (file, label) => {
  expect(source(file)).toContain(`accessibilityLabel="${label}"`);
});
```

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cd frontend
bun run test -- __tests__/components/weatherClearContracts.test.ts --runInBand
```

Expected: FAIL for any missing screen labels.

- [ ] **Step 3: Apply the minimum complete accessibility fix**

Add the exact labels from the test, roles for headings/buttons/tabs, selected and disabled states, and 44-point hit targets. Ensure graph summaries are textual and alert severity is written. Use `allowFontScaling` and flexible wrapping rather than fixed text heights.

- [ ] **Step 4: Check representative native widths and system font scale**

Run:

```bash
adb shell wm size 1080x2400
adb shell settings put system font_scale 1.0
adb exec-out screencap -p > /tmp/weather-clear-1080.png
adb shell wm size 720x1600
adb shell settings put system font_scale 1.3
adb exec-out screencap -p > /tmp/weather-clear-720-large-text.png
adb shell wm size reset
adb shell settings put system font_scale 1.0
```

Expected: no clipped primary controls, overlapping text, or unreachable tab items at either size.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cd frontend
bun run test -- __tests__/components/weatherClearContracts.test.ts --runInBand
bunx tsc --noEmit
```

Expected: PASS and exit 0.

Then:

```bash
git add frontend/src frontend/__tests__/components/weatherClearContracts.test.ts
git diff --cached --check
git commit -m "fix: complete native accessibility contracts"
```

## Task 10: Final Visual Capture, Full Verification, and Durable Notes

**Files:**

- Modify: `frontend/screenshots/01-home.png` through `05-settings.png`
- Modify: `docs/audits/2026-06-27-weather-clear-audit.md`

- [ ] **Step 1: Reset and capture the final five-screen Android set**

With the emulator restored to 1080 × 2400 and font scale 1.0, navigate each tab and save:

```bash
adb exec-out screencap -p > frontend/screenshots/01-home.png
adb exec-out screencap -p > frontend/screenshots/02-nowcast.png
adb exec-out screencap -p > frontend/screenshots/03-radar.png
adb exec-out screencap -p > frontend/screenshots/04-alerts.png
adb exec-out screencap -p > frontend/screenshots/05-settings.png
```

Before each command, verify the intended tab in `adb shell uiautomator dump` so a stale or loading screen is not mislabeled.

- [ ] **Step 2: Generate final target renders and visual comparisons**

Run:

```bash
mkdir -p /tmp/weather-clear-final
agent-browser --session weather-clear-reference open \
  "file:///home/vanillax/programming/radar-ng/Weather%20app%20redesign/Weather%20Clear.dc.html"
agent-browser --session weather-clear-reference set viewport 444 892
agent-browser --session weather-clear-reference screenshot /tmp/weather-clear-final/01-home-reference.png
agent-browser --session weather-clear-reference snapshot -i
```

Use the current refs shown by the snapshot to click Nowcast, Radar, Alerts, and Settings, saving `02-nowcast-reference.png` through `05-settings-reference.png`. After each click, take a fresh snapshot before the next click.

Normalize and combine each reference/runtime pair:

```bash
for name in 01-home 02-nowcast 03-radar 04-alerts 05-settings; do
  magick "/tmp/weather-clear-final/$name-reference.png" \
    -crop 384x832+30+30 +repage -resize 1080x2400\\! \
    "/tmp/weather-clear-final/$name-reference-normalized.png"
  magick "/tmp/weather-clear-final/$name-reference-normalized.png" \
    "frontend/screenshots/$name.png" +append \
    "/tmp/weather-clear-final/$name-side-by-side.png"
done
```

Expected: five target renders and five native captures arranged side by side. Do not report a pixel-difference score because native text rasterization, live data, and the normalized HTML frame are intentionally non-identical.

Update the audit matrix with resolved mismatches, intentional dynamic-data differences, dark-theme findings, and the iOS simulator limitation.

- [ ] **Step 3: Run complete fresh verification**

Run:

```bash
cd frontend
bun run test -- --runInBand
bunx tsc --noEmit
bun run lint
cd android
./gradlew :app:assembleDebug
```

Expected: all Jest suites pass with zero failures, TypeScript exits 0, lint exits 0, and the Android debug APK builds successfully.

- [ ] **Step 4: Inspect runtime logs**

Run:

```bash
adb logcat -c
adb shell am force-stop com.vanillax.radarng
adb shell monkey -p com.vanillax.radarng -c android.intent.category.LAUNCHER 1
adb logcat -d '*:E' ReactNativeJS:V AndroidRuntime:E
```

Expected: no `FATAL EXCEPTION`, React render error, missing-font error, or unhandled promise rejection from the tested flow.

- [ ] **Step 5: Save durable Mink notes**

Run:

```bash
mink note --project radar-ng --category projects \
  "Weather Clear reconciliation: Weather Clear.dc.html is the sole implementation target. uploads/01-home..05-settings and frontend/screenshots/ are byte-identical historical dark-app inputs, not target or runtime captures. App appearance is Light/Dark/System and is intentionally independent of radar map style. Android reference device is 1080x2400 at 420 dpi. See docs/audits/2026-06-27-weather-clear-audit.md."

mink note --project radar-ng --category resources \
  "Native visual verification runbook: render Weather app redesign/Weather Clear.dc.html with agent-browser at 444x892, crop the 384x832 phone at +30+30, normalize to 1080x2400, then compare with connected Android emulator com.vanillax.radarng. Verify route via uiautomator before screencap. Linux cannot provide final iOS simulator visual acceptance."
```

Record the returned note paths in the audit and final handoff.

- [ ] **Step 6: Commit final evidence**

```bash
git add frontend/screenshots docs/audits/2026-06-27-weather-clear-audit.md
git diff --cached --check
git commit -m "test: verify Weather Clear native reconciliation"
```

- [ ] **Step 7: Confirm the user’s original dirty change remains preserved**

Run:

```bash
git status --short
git diff -- frontend/src/stores/useWeatherStore.ts
git log --oneline -12
```

Expected: implementation commits are present, the original default-location logic is still represented exactly, and no unrelated user files were overwritten.
