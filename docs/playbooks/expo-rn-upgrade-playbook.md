# Playbook: big Expo / React Native upgrades with an LLM

Lessons from upgrading Radar NG (September 2026) through Expo SDK 57 → 58 preview, React Native 0.86 → 0.88, native tabs, Liquid Glass, and iOS 27 / Android 16. The work also covered a WidgetKit widget, a watchOS app and CarPlay. Written for an LLM that will carry out a similar upgrade on another app. Copy it into that repo's `AGENTS.md`/`CLAUDE.md`, or paste it as the first message.

---

## 0. Ground rules for the executor

1. **Audit first, then plan, then execute in small commits.** Write findings to a file with evidence (`file:line`, command output). Each task names how it will be verified. One commit per task, with a message that says *why*.
2. **Verify every claim yourself.** Versions, APIs and deadlines move weekly. Read the installed `node_modules/<pkg>/build/*.d.ts`, `bundledNativeModules.json` and `package.json` exports instead of trusting memory or blog posts. Mark anything you couldn't check as UNVERIFIED.
3. **Passing typecheck, lint and tests is not "done" for native work.** Several of the worst bugs below passed all three and only showed up in a **Release** build on a simulator or emulator. Budget for a native build and a UI pass after every dependency change.
4. **Never destroy what you haven't looked at.** `expo prebuild` recreates `ios/` and `android/` by default from SDK 57 on (`--no-clean` opts out). Before any prebuild, list those folders. People keep IPAs, keystores and exports there. Move build outputs to a gitignored `/artifacts/` so the native folders are truly disposable.
5. **Ask only for real decisions**: product calls, cost, credentials, anything irreversible or shared. Decide defaults yourself and say what you chose.

## 1. Order of operations

1. **Baseline.** Record the typecheck, lint and test results, the bundle size (`expo export` + source map) and a Release build, before touching anything.
2. **Hygiene and correctness fixes on the current SDK first.** They are cheaper to verify without a moving platform underneath.
3. **Replace source-grep tests with render tests *before* UI refactors.** Tests that `readFileSync` a component and `toContain("minHeight: 44")` break on every refactor and prove nothing. Add a second Jest project with the `jest-expo` preset and `@testing-library/react-native` that asserts on roles, labels and behaviour. Keep a mutation check: break the code on purpose and confirm the test fails.
4. **SDK bump on its own branch**, with lockfile, types and test-environment fixes.
5. **UI modernisation** (native tabs, glass, symbols) on top of the new SDK. Doing it on the old one means migrating twice.
6. **Native targets** (widgets, Watch, CarPlay) last. Each needs a real native build.
7. **Stacked PRs**: when a PR is based on another branch, merging it lands on *that branch*, not `main`. After the base merges, check that `git merge-base --is-ancestor feature origin/main` succeeds, or open a follow-up PR.

## 2. Expo SDK upgrade checklist

- `bun add expo@<target>` (or npm/yarn), then `bunx expo install --fix`, then `bunx expo install --check` until clean, then `bunx expo-doctor`.
- **Upgrading in place can leave duplicate copies of core packages** (`expo-constants`, `expo-font`). If doctor reports duplicates, delete `node_modules` and reinstall; that fixed it here.
- Check **non-Expo native dependencies** yourself: MapLibre, MMKV, Nitro modules, Skia, `@bacons/apple-targets`, anything with native code. Compare `npm view <pkg> peerDependencies` and the release notes against the new React Native version.
  - Real example: `react-native-nitro-modules` below 0.36.2 aborts at launch on RN 0.87+ with `installJSIBindingsWithRuntime was not called`.
  - It was only a transitive dependency (through MMKV), so pin it directly.
- Expect small API moves. Grep for every import path of a package after the bump.
  - Example: SDK 58 exports `useIsFocused` from `expo-router`, and `expo-router/react-navigation` no longer resolved in Jest.
- Type drift, for example RN 0.88's `AppState.currentState` becoming `string | undefined`. Widen the types; don't cast them away.
- Drop stale `postinstall` patches and config-plugin workarounds after checking that the upstream package now ships the fix (diff the pristine tarball from `npm pack`).
- Previews and betas are fine if the owner accepts them, but pin exact versions and note every preview workaround in code comments so it can be removed later.

## 3. Jest after the upgrade

- Run two projects:
  - `unit`: ts-jest, node environment, fast, for pure logic and config plugins.
  - `render`: the `jest-expo/ios` preset. Scope `babel-preset-expo` inside that project's `transform`, so you don't need a `babel.config.js` that also changes the app build.
- **Testing Library 14 made `render` and `fireEvent` async.** `await` them; otherwise you get "render function has not been called".
- **Preview jest-expo added an `expo-source` export condition that points at `src/` files the published packages don't ship.** Set `testEnvironmentOptions.customExportConditions: ["react-native"]` in the render project.
- A React Query client in tests needs `gcTime: Infinity` plus `client.clear()` in `afterEach`, or Jest never exits and CI hangs.
- Pass providers through Testing Library's `wrapper` option so `rerender` keeps them.
- `jest.mock` factories may only reference variables prefixed `mock`.
- Mock native-only modules in one setup file: MMKV (in-memory), Reanimated and Worklets mocks, safe-area mock, SwiftUI or `@expo/ui` sheets (render children), `expo-symbols`, `expo-glass-effect`.

## 4. Native tabs (expo-router `NativeTabs`)

- Stable at `expo-router/native-tabs` from SDK 58 (`unstable-native-tabs` before that).
- Triggers use `<NativeTabs.Trigger.Icon sf={{ default: "house", selected: "house.fill" }} md="home" />`. Both SF Symbol and Material names are **type-checked**, so let `tsc` validate them.
- Hide the bar on a full-screen route with `hidden={segments.at(-1) === "radar"}` from `useSegments()`.
- **Android Material hides inactive labels when there are more than three tabs.** Set `labelVisibilityMode="labeled"` if tests or users rely on labels.
- The preview ignored `Badge hidden` and showed "0". Render the badge only when there's a count.
- A custom JS tab bar was an absolute overlay, and screens padded their bottoms by about 120 pt to clear it. Native tabs inset content themselves, so remove that padding and check on device.

## 5. Liquid Glass (iOS 26+)

- `expo-glass-effect`: `GlassView` behind `isLiquidGlassAvailable()`, and turn it off when `AccessibilityInfo.isReduceTransparencyEnabled()` (subscribe to `reduceTransparencyChanged`).
- Wrap it in one component, for example `<ChromeSurface style fallbackStyle colorScheme tintColor interactive>`. `style` holds shape and layout (applied always); `fallbackStyle` holds fill, border and shadow (applied only without glass). Split existing styles into those two groups.
- **Pin `colorScheme` for each surface** to match its text colour: light cards with dark text use `"light"`, dark controls with white text use `"dark"`. Otherwise glass follows the system appearance and text loses contrast.
- Glass over busy, bright content (maps) can wash out. A `tintColor` such as `rgba(15,18,30,0.6)` restores contrast. You only find this by looking at a screenshot.
- iOS 27 ignores the `UIDesignRequiresCompatibility` opt-out, so plan for glass rather than avoiding it.

## 6. Icons

- Replace hand-drawn View icons with `expo-symbols` `SymbolView` using `name={{ ios: "…", android: "…" }}`. Material names are validated against `expo-symbols/build/android/symbols.json`. Here this cut a controls file from about 770 lines to 340 and fixed Dynamic Type weight.
- Keep animation wrappers (for example a spinning refresh icon) around the symbol, not inside it.

## 7. Layout and accessibility

- **Absolutely positioned children ignore their `SafeAreaView` parent's padding.** Compute offsets from `useSafeAreaInsets()`. Make it one hook that returns `{top, bottom, left, right}` edges for chrome, tuned so a notched iPhone looks the same as before; Android status and navigation bars and landscape cutouts then get correct room.
- Cap Dynamic Type only on dense overlay chrome (`maxFontSizeMultiplier` ≈ 1.3), not on sheets or screens.
- Honour Reduce Motion for autoplay.
- Let a full-screen map set its own status-bar style, only while its tab is focused.
- Keep map attributions (OpenStreetMap and others) visible. Overlays hide them easily.

## 8. React Compiler

- If `app.json` enables it, **turn the React Compiler lint rules back on** (`react-hooks/refs`, `purity`, `immutability`, `set-state-in-effect`) and fix what they find. Then run `bunx react-compiler-healthcheck`.
- Typical fixes:
  - `Date.now()` in render → a `useNow(intervalMs)` hook.
  - `useRef(new X()).current` → `useState(() => new X())[0]`.
  - setState in an effect body → derive the value, or key it to its inputs.
  - A ref written during render → state updated during render from the previous value.
- Code that intentionally mutates on the UI thread (Reanimated or Skia buffers) gets per-line disables with a reason, not a global off switch.

## 9. Native targets (widget, Watch, CarPlay)

- `@bacons/apple-targets` 5 writes entitlements to `ios/.targets/<name>/generated.entitlements`. Old `targets/*/generated.entitlements` files are dead, so delete them to avoid confusion.
- Sharing settings with extensions:
  - Widgets and CarPlay use an **App Group** (entitlement on the app and on each extension), with UserDefaults(suiteName:).
  - **App Groups don't cross devices, so the Watch needs WatchConnectivity** (`updateApplicationContext`, and read `receivedApplicationContext` on activation).
  - A small local Expo module in `modules/<name>` can do both, from one JSON payload.
- Keep one decoder file identical across targets, and add a test that the copies match and that the App Group id agrees with `app.json`.
- Build formatters once (`ISO8601DateFormatter`, `DateFormatter`); constructing them per parse is slow inside `filter` or `max` over large lists.

## 10. Android specifics

- **16 KB page size**: check the APK with `zipalign -c -P 16 -v 4` and `llvm-objdump -p` on each `.so`. Every `LOAD` segment should be `2**14`.
- Move `usesCleartextTraffic` into a Network Security Config via a config plugin; the attribute is ignored from API 38.
- Add `adaptiveIcon.monochromeImage` for themed icons. If the foreground is opaque, derive the monochrome layer from luminance.
- Local release signing without secrets in git:
  - the upload key lives in `~/.<app>/`;
  - a config plugin reads `findProperty('X_UPLOAD_*')`;
  - the build script exports `ORG_GRADLE_PROJECT_X_UPLOAD_*` environment variables;
  - raise the Gradle heap (`-Dorg.gradle.jvmargs=-Xmx4g`), because the default 2 GB can fail dex merging.
- `expo-location` on Android raises Google's "Location Accuracy" dialog by default. Pass `mayShowUserSettingsDialog` only for user-initiated location requests, or users get nagged on every foreground.

## 11. End-to-end tests (Maestro) across both platforms

- Parameterise `appId: ${APP_ID}` and pass `-e APP_ID=…` from per-platform runner scripts. iOS and Android bundle IDs often differ.
- **Android shows the runtime permission prompt even after Maestro's `permissions: all: deny`**, and Google Play Services raises the Location Accuracy dialog. Put optional dismissals in a shared `common/dismiss-system-prompts.yaml`, run it after every `launchApp`, and keep the fix in the app where it's a real UX problem (see §10).
- The Android keyboard covers bottom buttons. Use `runFlow: when: platform: Android` with `hideKeyboard` before tapping them.
- Simulator permission state leaks between runs. Run `xcrun simctl privacy <device> reset location <bundle>` before permission-sensitive flows.
- A leftover `simctl openurl` confirmation dialog blocks the next flow. Tap `Cancel` optionally before `launchApp`.

## 12. Visual verification loop (do this, it found half the bugs)

1. Build **Release** for the simulator: `xcodebuild … -configuration Release … CODE_SIGNING_ALLOWED=NO`. The JS bundle is embedded, so no Metro is needed and startup crashes surface.
2. `xcrun simctl install`, launch, and wait. If the home screen shows instead of the app, it crashed. Read `~/Library/Logs/DiagnosticReports/<app>-*.ips`, then `simctl spawn <device> log show --predicate 'process == "<app>" AND category == "javascript"'` for the JS error behind `RCTFatalException`.
3. Screenshot every tab with a Maestro flow (`takeScreenshot`) and actually look at them: contrast, overlaps, hidden attributions, stray badges.
4. On Android: `adb install`, `adb exec-out screencap -p`, and `adb logcat` for `FATAL|AndroidRuntime`.

## 13. When data looks wrong, check the backend before "fixing" the UI

After making the UI honest about missing values (showing "—" instead of 0°, cloudy or dry), the app showed "Conditions unavailable" everywhere. Root cause: the self-hosted Open-Meteo sync requested variable names the API **derives** at query time (`weather_code`, `wind_speed_10m`, `apparent_temperature`), so nothing was stored. Lessons:

- Inspect the live API response and the upstream data store directly: `kubectl exec … ls /app/data/<model>`.
- Reproduce the fix locally with the exact pinned upstream image before opening the PR.
- Capacity-plan storage for the change, and ship the infra PR alongside it.

## 14. Deliverables the owner actually wants

- A plan file with a progress table at the top, updated as tasks land.
- PRs with verification sections that say exactly what ran and what didn't (for example "no physical device yet").
- Short notes of durable gotchas, like the ones in this file, saved where the next session will find them.
