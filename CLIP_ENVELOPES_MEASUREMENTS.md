# Playlist VOL/PAN rendering measurements

Measured optimized Linux synthetic UI workload, commit `4bc668c`, Actions run [44783609173](https://github.com/jellyscoll76-Real/honeydew/actions/runs/44783609173). The benchmark job passed; the regression job did not, so this is a rendering measurement, not release acceptance.

Eight cached waveform meshes, with/without both envelope lanes; 64 egui frames and 64 CPU tessellations per repeat, median of three repeats. Fonts and curve caches warmed first. Mesh clone/translate and egui frame overhead included equally in both cases.

| Controls per lane | Both lanes | Paint/frame ms | CPU tessellation ms | Vertices | Indices |
|---|---|---:|---:|---:|---:|
| 2 | No | 0.0274 | 0.0657 | 20,480 | 30,720 |
| 2 | Yes | 0.1637 | 0.2743 | 44,384 | 136,224 |
| 32 | No | 0.0260 | 0.0666 | 20,480 | 30,720 |
| 32 | Yes | 0.5649 | 0.5357 | 82,144 | 300,384 |
| 128 | No | 0.0257 | 0.0618 | 20,480 | 30,720 |
| 128 | Yes | 1.2733 | 1.2576 | 204,320 | 831,744 |

Both lanes add approximately 0.345 / 1.008 / 2.443 ms of combined painting and CPU tessellation per frame for 2 / 32 / 128 controls per lane in this workload. Drawing is not free; dense control-point markers and curve geometry are appreciable overhead. Cache bounds avoid unlimited retained curves, but do not eliminate tessellation costs.

These are not phone FPS, GPU timings, whole-app CPU, battery, or heavy-project responsiveness results. Physical Android playback/editing checks remain required. Source audio is not rewritten by envelope editing; waveform gain is applied to cached summaries while drawing.
