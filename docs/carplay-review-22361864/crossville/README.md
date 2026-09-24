# Crossville radar and navigation captures — September 22, 2026

These replace the Grand Rapids close-ups for demonstrating visible radar.
Captured directly from the iOS simulator, without editing the images, using the
shared Radar NG map controller and tile overlay in the development review app.
The map spans approximately 65 km around Crossville, Tennessee, west of Knoxville.
Live MapKit directions lead to Knoxville Convention Center along I-40 East.
GPS is replayed and labelled; this remains an iPhone development presentation,
not the CarPlay system interface or an in-vehicle test.

The screenshots visibly show green, yellow, and orange radar returns over the
route. The live server observation used is `2026-09-22T04:29:59+00:00`, displayed
as 12:29 AM Eastern. Tile diagnostics recorded 16 delivered tiles, 14 containing
nontransparent radar pixels, at each capture. The counts are cumulative for the
frame and are not a claim that every delivered tile is fully inside the viewport.
Visual inspection separately confirmed the radar appears in all three PNGs.

- [Approaching the roundabout](01-approaching-turn.png): 300 m / 0.2 miles.
- [Near the same roundabout](02-near-turn.png): 40 m / 150 feet.
- [Next maneuver](03-next-maneuver.png): right merge onto I-40 East, about 515 m / 0.3 miles.

[Recorded metadata and tile counts](maneuver-evidence.json) · [Runtime checks](review-result.txt)

The review host now supports location, destination, map scale, and a requirement
for nontransparent radar tile delivery. Captures fail if that requirement is
enabled and no radar echoes arrive. A frame timestamp alone does not satisfy it.
The production maneuver symbol now recognizes MapKit's English “At the roundabout,”
instruction and uses a generic circulation arrow instead of a misleading straight
arrow derived from the entrance tangents. This is not exit-specific roundabout
geometry or complete localized roundabout support. Regression checks passed.

Reproduce from the repository root (the live weather will change):

```sh
REVIEW_PLATFORM=simulator \
REVIEW_DEVICE_UDID=0F62AE0E-43D1-4EB5-ADB5-CA2A858A80DB \
RADAR_REVIEW_CITY=Crossville \
RADAR_REVIEW_LATITUDE=35.9489 \
RADAR_REVIEW_LONGITUDE=-85.0269 \
RADAR_REVIEW_DESTINATION='Knoxville Convention Center' \
RADAR_REVIEW_MAP_METERS=65000 \
RADAR_REVIEW_REQUIRE_RADAR=1 \
bash frontend/scripts/capture-carplay-review.sh
```

Release build, maneuver progression, stale GPS rejection, End Trip, route/symbol
regressions, and radar tile delivery checks passed. Actual CarPlay session and
vehicle validation remain pending. No email was sent.
