# Android QA — 2026-09-24

First native Android build (plan tasks 6.1 and 6.2). The build compiled; nothing has been installed or run on an emulator or device yet.

## Build

- Tree: branch `mobile/phase-0-prepare` at `cff6a0b`.
- Method: `bunx expo prebuild --platform android --no-install` in a **scratch copy** of `frontend/` (native folders excluded, `node_modules` symlinked), then `./gradlew assembleRelease`. The real `frontend/android` was not regenerated.
- Toolchain: Gradle 9.3.1, JDK 21 (Temurin), Android SDK with build-tools 35+.
- Output: `app-release.apk`, 195 MB (four ABIs, universal, signed with the template debug keystore). `com.vanillax.radarng`, versionCode 3, versionName 2.0.0, compileSdk 36, targetSdk 36.
- Config and plugin changes needed to compile: **none**. Warnings are upstream deprecations (MapLibre, safe-area-context, screens, masked-view, slider).
- One transient failure: the first full build failed in `:app:mergeDexRelease` with an empty `DexArchiveMergerException` while other heavy work ran on the machine. Re-running the task succeeded without changes. If it recurs, raise `org.gradle.jvmargs` above the generated `-Xmx2048m`.

The generated manifest carries `android:networkSecurityConfig="@xml/network_security_config"` and no `usesCleartextTraffic`. The adaptive icon includes the monochrome layer.

## 16 KB page size (6.2)

- `zipalign -c -P 16 -v 4`: all 108 `.so` entries (27 libraries × 4 ABIs) pass.
- ELF `LOAD` segment alignment (`llvm-objdump -p`, arm64-v8a and x86_64): 164 of 164 segments are `2**14` (16 KB). That includes `libmaplibre.so`, `librnskia.so`, `libmmkv.so`, `libworklets.so` and `libreanimated.so`. MMKV, previously unverified, complies.

## Not yet done

- Install and run: `scripts/test-android-emulator.sh` (Maestro flows with `APP_ID=com.vanillax.radarng`). Flow text written for iOS may need Android adjustments.
- 6.5 predictive back and 6.6 tablet/foldable/landscape need an emulator session.
- Release signing and AAB (6.8, decision D5).

## targetSdk 37 readiness (don't bump yet)

- **Local network access.** Android 17 gates LAN connections behind `ACCESS_LOCAL_NETWORK` for apps targeting 37. Users who point the stack URL at a LAN server will need it; request it only when the configured host is private.
- **Cleartext.** Already moved to a Network Security Config, so the API 38 removal of `usesCleartextTraffic` doesn't affect LAN http.
- **Certificate Transparency.** Enforced by default for newer targets. The self-hosted endpoints use publicly trusted certificates; user-installed CAs are not trusted by the config.
- **Large screens.** The opt-out from resizability/orientation restrictions on large screens is removed for newer targets; covered by 6.6.
