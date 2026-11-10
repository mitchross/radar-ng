# Replay importer pilot evidence

The pilot imports a bounded archive slice into local scratch storage.
Full cases and production storage are outside this check.

## Archive probes

`radar replay check` listed real MRMS keys in both windows: 90 objects for
Greenfield base reflectivity and 120 for Rolling Fork, both at a nominal 120 s
cadence. The first base-reflectivity keys are `20230324-230015` (Rolling Fork)
and `20240521-190017` (Greenfield).

## Bounded import

Command per case, run on an optimized build from the branch:

```sh
radar replay import --case <case> --revision pilot-1 \
  --tile-root /opt/data/cache/scratch/replay-pilot-release/tiles \
  --max-frames 1 --verify-crop
```

`--max-frames 1` imports one frame for each of the six layers. `--verify-crop`
renders the same tiles from the uncropped grid and stops on any byte difference,
so a passing run proves boundary parity.

| Case | Wall time | Render time (6 frames) | Peak RSS | Boundary PNGs compared | Allocated | PNG files |
|---|---:|---:|---:|---:|---:|---:|
| rolling-fork-2023 | 17.4 s | 1.03 s | 2.40 GiB | 96 | 1.20 MB | 116 |
| greenfield-2024 | 15.9 s | 0.72 s | 2.38 GiB | 102 | 1.18 MB | 122 |

Both runs exited 0. Source GRIBs stay in memory for one frame and leave no
full-CONUS files on disk. The 10 GiB replay quota and the 20 GiB live headroom
check passed before every frame.

## No live side effects

The pilot ran against a scratch tree holding a live manifest, a nowcast sentinel
and a live tile sentinel. Hashes of every file outside `replays/` were identical
before and after both imports. Nothing wrote the live manifest, nowcast inputs or
the live tile root. A separate test proves the live cleanup sweep ignores
`replays/`.
