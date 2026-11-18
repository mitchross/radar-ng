# Phone day: real-device acceptance

One checklist for Mitch. You build each phone app once, then work through three parts.
Nothing here is ticked by an agent. Tick a box only after you saw it on the phone.

The checkboxes copy the GitHub issues word for word: [#252](https://github.com/mitchross/radar-ng/issues/252) (part A), [#285](https://github.com/mitchross/radar-ng/issues/285) (part B), [#293](https://github.com/mitchross/radar-ng/issues/293) (part C). Tick the box on the issue too.

**Not run on a device.** The Maestro flows `06-fast-radar.yaml` and `07-replay.yaml` and the script `android-inmap-evidence.sh` were written without a phone or emulator. Expect to fix a label or a timing the first time. If one fails, do the step by hand.

| Part | Issue | When |
|---|---|---|
| A. Fast radar on Android | #252 | Any time. |
| B. Famous storm replay on iOS and Android | #285 | After both replay cases are published on the server you point the app at. |
| C. Level II close-up on iOS and Android | #293 | **Run only after the Level II shadow host exists.** Skip it until then. |

## Before you start

- [ ] Android phone plugged in, USB debugging on, `adb devices` lists it.
- [ ] iPhone plugged in and trusted by this Mac (parts B and C only).
- [ ] Phone is on the server you want to test (Settings, Adv, stack URL). For part B it must list both storms. For part C it must be the shadow host.

### Build the Android app

From `frontend/`:

```bash
bunx expo prebuild --platform android --clean --no-install
bunx expo run:android --variant release   # device plugged in, or an emulator
```

A signed APK to sideload is `bun run android:apk` ([android-release.md](android-release.md)). It needs the upload key from that page.

### Build the iOS app

From `frontend/`:

```bash
bun run ios:ipa
```

Install the IPA on the iPhone. Setup is in [running-the-app.md](running-the-app.md).

### Collect Android evidence

The script saves logs and screenshots to a folder. Run it from `frontend/`. Set `ANDROID_SERIAL` if more than one device is plugged in.

```bash
bash scripts/android-inmap-evidence.sh clear             # right before you start
bash scripts/android-inmap-evidence.sh screenshot NAME   # any time you want a picture
bash scripts/android-inmap-evidence.sh collect           # when you finish, or the moment something fails
```

It prints the folder. Attach `results.txt`, `logcat-filtered.txt` and the screenshots to the issue.

### Optional: the Maestro flows

Install [Maestro](https://maestro.mobile.dev). From `frontend/`:

```bash
APP_ID=$(node -p "require('./identity').androidPackage")
maestro --device "$ANDROID_SERIAL" test -e APP_ID="$APP_ID" .maestro/06-fast-radar.yaml
maestro --device "$ANDROID_SERIAL" test -e APP_ID="$APP_ID" -e REPLAY_CASE=Greenfield .maestro/07-replay.yaml
```

For iOS use the bundle id (`node -p "require('./identity').bundleId"`) and the iPhone's id from `maestro --help`. They check text on screen only. They cannot judge smoothness or overlay order.

---

## Part A. Fast radar on Android (#252)

Do this on rain. If it is dry, pick a rainy city in Settings. Run `clear` of the evidence script first.

- [ ] Turn on Fast radar over rain. Radar draws at zooms 5, 6, 7 and 8.
  1. Open Settings. Find **Fast radar (GPU)**. Make sure it is on.
  2. Open the Radar tab. Pan to rain.
  3. Zoom to 5. Look for radar. Then 6, 7, 8.
  4. Take a screenshot at each zoom.
- [ ] The pin, warning outlines and storm tracks draw **above** the radar.
  1. Long-press the map to drop a pin.
  2. Open the layer button, turn on **Storms & lightning**.
  3. Find a warning outline or a storm track over the radar.
  4. Check the outline and track are on top of the colours, not under.
  5. Take a screenshot.
- [ ] Playback is smooth, with no blank frames.
  1. Tap **Play radar animation**.
  2. Watch two full loops.
  3. Note any blank frame or jump. Take a screenshot if you see one.
  4. Tap **Pause radar animation**.
- [ ] After a pan, radar fills back in quickly.
  1. Drag the map about one screen to a new spot with rain.
  2. Count the seconds until radar fills the screen.
  3. Write the number down.
- [ ] The status line under Fast radar shows frames and tiles counting up, and `no errors`.
  1. Open Settings. Look under **Fast radar (GPU)**.
  2. Read the line. It starts `In-map: N frames · N tiles`.
  3. Go back to the radar, zoom or pan, return to Settings. The numbers should be higher.
  4. The second line should say `no errors`.
  5. Copy both lines into the issue.
- [ ] Air Quality and Wind layers: no crash.
  1. On the Radar tab, open **Choose radar layer**.
  2. Pick **Air Quality**. Wait 10 seconds.
  3. Pick **Wind**. Wait 10 seconds.
  4. Pick **Radar** again.
- [ ] Kill and relaunch the app. Radar draws again.
  1. Swipe the app away from the app switcher.
  2. Open it. Go to the Radar tab.
  3. Wait up to 15 seconds for radar to draw.

### Also check: the fallback (not a #252 checkbox, but #252 says it should happen)

The map should never go blank. If Fast radar cannot draw, the normal raster radar shows after about 8 seconds.

- [ ] If Fast radar failed, the normal radar appeared and the map was not blank.
  - The status line second line then reads `fallback: ...`. Copy it.

### If something fails

Run `bash scripts/android-inmap-evidence.sh collect` straight away, then add:

- the status line text, which shows the error stage and the fallback reason;
- `adb logcat | grep -iE "radar|maplibre|inmap"` (the script saves the same lines);
- the tile files in the app's cache folder `radar-inmap` (the script saves them only on a debug build; on a release build it says so);
- PostHog `inmap_diag` events.

---

## Part B. Famous storm replay, iOS and Android (#285)

Run only after both storms are published. Do the whole list once for each of the four pairs:

- [ ] iOS, Greenfield
- [ ] iOS, Rolling Fork
- [ ] Android, Greenfield
- [ ] Android, Rolling Fork

The #285 checkbox these four cover:

- [ ] Same on real iOS and Android builds, including a memory/prefetch check and notification isolation

Steps for each pair:

1. Open the Radar tab. Tap **Famous storms**. Tap the storm.
   - You should see the **HISTORICAL REPLAY** banner with the storm name.
   - If **Famous storms** is missing, the server lists no storms. Stop and check the stack URL.
2. **Load.** Wait for radar. No red "Could not load this replay" line.
3. **Scrub.** Drag the timeline back and forth. The picture follows the finger.
4. **Change layer.** Open **Choose radar layer**. Pick a layer, then another. The time stays where it was.
5. **Loop.** Tap **Play radar animation**. Watch a full loop. Tap **Pause radar animation**.
6. **Slow network.** Make the phone slow, then scrub and play again.
   - iPhone: Settings, Developer, Network Link Conditioner, pick "3G" or worse.
   - Android has no switch for this. Use a throttled hotspot, or stand where signal is weak.
   - Pass: it waits or shows a message. It does not crash or stay blank for good.
7. **Missing data.** Scrub across the gaps recorded for the storm. The app should show no radar there, or a clear message, and recover when you scrub on.
8. **Return to live.** Tap **Return to live**. The banner goes. Live radar and live overlays come back.
9. **Memory and prefetch.**
   - Android: run `adb shell dumpsys meminfo $(node -p "require('./identity').androidPackage")` before replay, after one full loop, and after Return to live. Write down the "TOTAL PSS" number each time.
   - iPhone: watch the memory gauge in Xcode (Debug navigator) at the same three points.
   - Pass: memory grows during replay, then falls back near the first number after Return to live. It must not keep climbing every loop. The design keeps a bounded lookahead, not every frame in memory.
   - Prefetch: after Return to live, the app should stop asking for replay tiles. Watch the server's request log for `/replays/` paths.
10. **Notification isolation.** Replay storms contain real tornado and severe warnings.
    - Allow notifications for the app.
    - While the replay plays, no warning notification may arrive for the replayed warnings.
    - Real live warnings are separate and may still arrive.
    - Pass: zero notifications caused by the replay. Check the notification shade at the end.

Maestro can cover steps 1, 2, 5, 8 on one storm: `07-replay.yaml` (see above).

The other #285 checkboxes are not phone checks. They are done on web and on the server:

- Both cases imported with recorded coverage, gaps, checksums and allocated bytes
- Operator import and rebuild documented
- Browser run: load, scrub, change layer, loop, slow network, missing data, return to live
- Publish only after both pass

When part B passes on both phones, close #285 and tick "At least two cases replay on web and app" on [#262](https://github.com/mitchross/radar-ng/issues/262).

---

## Part C. Level II close-up, iOS and Android (#293)

**Run only after the Level II shadow host exists.** Until then Level II is off everywhere and there is nothing to see.

Setup:

1. Build and install fresh iOS and Android apps (see above). Older builds have no close-up.
2. In Settings, Adv, set the stack URL to the shadow host.
3. Pick a pilot site near you, or use a region with a pilot site.
4. Rain or snow over that site makes the test easier. If the day is clear, only the fallback and range parts can be tested.

Close-up facts you are checking against (from the design): below zoom 8 you see the normal radar; between 8 and 9 the close-up fades in; from zoom 9 up it is fully on. The nearest working station within 150 km of the view centre is used, one station at a time. A sweep must be no more than 10 minutes old.

The #293 checkbox these steps cover:

- [ ] Real browser and rebuilt iOS/Android apps pass six-sweep playback, time alignment, station/range/age transitions, provenance, colour-vision modes, slow/offline network and MRMS fallback. Publish evidence, not synthetic benchmarks or JS-only phone claims.

The browser part is not yours. Agents run it. Do the apps, once on iOS and once on Android:

- [ ] iOS
- [ ] Android

1. **Six-sweep playback.**
   - Zoom to 9 or closer over the site.
   - Tap **Play radar animation**.
   - Count the frames. There should be six sweeps (one is the newest).
   - They play in time order. No blank frame, no jump back.
2. **Time alignment.**
   - Pause on a frame. Read the time on the close-up note and on the timeline.
   - They should match. The picture under it (normal radar) should be from about the same minute.
3. **Station, range and age transitions.**
   - Zoom out from 9 to 7. The close-up fades out. Normal radar stays.
   - Zoom back in. It fades in.
   - Pan more than 150 km from the site. The close-up goes away and normal radar stays. A short notice may explain why.
   - If a sweep is older than 10 minutes, the close-up should drop away and normal radar shows. You may not be able to make this happen. Write "not seen" if so.
4. **Provenance.**
   - Tap the note at the top of the map. It opens to show station, scan time, tilt and beam height.
   - The values should look right: the station you expect, a time within minutes, tilt about 0.5.
   - Take a screenshot.
5. **Colour-vision modes.**
   - Settings, radar palette: **Classic**, **Vivid**, **Muted** (colourblind-safe).
   - For each, compare close-up colours to the normal radar beside it at zoom 8.5. They must use the same palette.
6. **Slow and offline network.**
   - Slow: as in part B step 6, then zoom in.
   - Offline: turn on airplane mode while zoomed in. The map should keep the last picture or fall back. It must not crash.
   - Turn the network back on. Close-up returns.
7. **MRMS fallback.**
   - If you can, ask whoever runs the shadow host to switch Level II off for your site, or pick a site that is not in the pilot.
   - Zoom in. Normal radar stays. A note says why. The map is never blank.

Take a screenshot at each step. Name them `c-ios-1.png`, `c-android-1.png` and so on, and attach them to #293. Do not paste synthetic numbers.

The other #293 checkboxes are not phone checks (shadow run, speed numbers, load and failure drills, meteorological review, final report). They are not on this page.

---

## What the Maestro flows cover

| Flow | Drives | Left to you |
|---|---|---|
| `06-fast-radar.yaml` | Fast radar on, status line shows frames and `no errors`, no fallback line, Air Quality and Wind, kill and relaunch | Zoom 5 to 8, overlays above radar, smoothness, fill-in speed after a pan |
| `07-replay.yaml` | Open a storm, banner shows, play, pause, layer sheet, Return to live | Scrub feel, slow network, missing data, memory, notifications |

Neither flow touches Level II (part C). Level II needs the shadow host and a human eye.
