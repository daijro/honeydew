#!/usr/bin/env python3
"""Synthetic UI mesh timings, not Android FPS, playback latency, or GPU execution time."""
import json
import pathlib
import re
import statistics

root = pathlib.Path('benchmark-results')
rows = []
for line in (root / 'waveforms.log').read_text().splitlines():
    if 'WAVE_BENCH ' in line:
        rows.append(dict(re.findall(r'(\w+)=([^ ]+)', line.split('WAVE_BENCH ', 1)[1])))
assert len(rows) == 36, f'Expected 36 measurements, got {len(rows)}'
report = ['# Waveform quality — controlled release measurements', '',
          'Linux synthetic UI benchmark: one 32-second 48 kHz stereo sample, eight visible clip meshes. '
          'Three repeats per mode and workload; medians below. Decode/summary construction excluded. '
          'Cold = 64 batches of eight mesh builds with geometry cache cleared per batch; warm = 256 '
          'batches of cache lookups; tessellation = 128 egui CPU tessellations of eight cached meshes. '
          'Times are wall-clock per batch, not Android frame rates, GPU draw execution, touch latency or full-app CPU utilization.', '',
          '| View | Mode | Cold build µs/8 clips | Warm lookup µs/8 clips | CPU tessellation µs/8 clips | Vertices | Triangles | Geometry bytes |',
          '|---|---|---:|---:|---:|---:|---:|---:|']
results = {}
for case in ('close', 'medium', 'overview', 'hidpi'):
    modes = {}
    for mode in ('High', 'Balanced', 'Performance'):
        runs = [r for r in rows if r['case'] == case and r['mode'] == mode]
        assert len(runs) == 3
        median = {key: statistics.median(float(r[key]) for r in runs) for key in
                  ('cold_512_ms', 'warm_2048_ms', 'tessellate_128_ms', 'vertices', 'indices', 'geometry_bytes')}
        modes[mode] = {'runs': runs, 'median': median}
        report.append(f'| {case} | {mode} | {median["cold_512_ms"] * 1000 / 64:.2f} | '
                      f'{median["warm_2048_ms"] * 1000 / 256:.2f} | {median["tessellate_128_ms"] * 1000 / 128:.2f} | '
                      f'{median["vertices"]:.0f} | {median["indices"] / 3:.0f} | {median["geometry_bytes"]:.0f} |')
    a, b, c = [modes[m]['median'] for m in ('High', 'Balanced', 'Performance')]
    assert a['vertices'] > b['vertices'] > c['vertices'], f'{case}: modes must reduce actual geometry'
    description = []
    for mode in ('High', 'Balanced', 'Performance'):
        m = modes[mode]['median']
        description.append(f'{mode}: vertices={m["vertices"]:.0f}, cold_us={m["cold_512_ms"]*1000/64:.2f}, '
                           f'warm_us={m["warm_2048_ms"]*1000/256:.2f}, tess_us={m["tessellate_128_ms"]*1000/128:.2f}')
    print('::notice title=Waveform measurement ' + case + '::' + '; '.join(description))
    results[case] = modes
report += ['', 'All modes share the unchanged source PCM and peak pyramid. Tests verify display bounds, '
           'conservative transient preservation, reversal, stereo, mode-specific cache reuse and settings persistence. '
           'No physical Android performance or audible-sync claim is supported by this benchmark.']
(root / 'measurements.json').write_text(json.dumps(results, indent=2))
(root / 'REPORT.md').write_text('\n'.join(report) + '\n')
