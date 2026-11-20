#!/usr/bin/env python3
"""Summarize isolated release benchmark processes; never compare unlike codec settings."""
import json
import pathlib
import re
import statistics

root = pathlib.Path('benchmark-results')
results = {}
for fmt in ('wav', 'flac', 'mp3', 'ogg'):
    modes = {}
    for mode in ('baseline', 'candidate'):
        rows = []
        for repeat in (1, 2, 3):
            log = (root / f'{fmt}-{mode}-{repeat}.log').read_text()
            bench = next(line for line in log.splitlines() if line.startswith('BENCH '))
            row = dict(re.findall(r'(\w+)=([^ ]+)', bench))
            profile = next(line for line in log.splitlines() if line.startswith('export-profile '))
            row.update(dict(re.findall(r'(\w+)=([^ ]+)', profile)))
            resource = (root / f'{fmt}-{mode}-{repeat}.resources').read_text()
            for key, pattern in {
                'process_peak_rss_kib': r'Maximum resident set size \(kbytes\): (\d+)',
                'process_cpu_percent': r'Percent of CPU this job got: (\d+)%',
                'process_user_s': r'User time \(seconds\): ([\d.]+)',
                'process_system_s': r'System time \(seconds\): ([\d.]+)',
            }.items():
                row[key] = re.search(pattern, resource).group(1)
            rows.append(row)
        modes[mode] = {
            'runs': rows,
            'median': {key: statistics.median(float(row[key]) for row in rows) for key in (
                'total_export_s', 'compute_s', 'write_s', 'prepare_s', 'analysis_s',
                'processing_s', 'render_s', 'encode_s', 'flush_s', 'process_peak_rss_kib',
                'process_cpu_percent', 'process_user_s', 'process_system_s')},
        }
    a, b = modes['baseline'], modes['candidate']
    pcm_match = len({r['pcm_fnv64'] for m in modes.values() for r in m['runs']}) == 1
    size_match = len({r['encoded_bytes'] for m in modes.values() for r in m['runs']}) == 1
    speedup = a['median']['total_export_s'] / b['median']['total_export_s']
    results[fmt] = dict(modes=modes, pcm_fingerprints_match=pcm_match,
                        encoded_sizes_match=size_match, median_speedup=speedup)
    summary = (f'{fmt}: baseline={a["median"]["total_export_s"]:.4f}s '
               f'candidate={b["median"]["total_export_s"]:.4f}s speedup={speedup:.3f}x '
               f'PCM_fingerprints_match={pcm_match} sizes_match={size_match} '
               f'peak_RSS_KiB={a["median"]["process_peak_rss_kib"]:.0f}/{b["median"]["process_peak_rss_kib"]:.0f} '
               f'process_CPU_percent={a["median"]["process_cpu_percent"]:.0f}/{b["median"]["process_cpu_percent"]:.0f} '
               f'analysis_s={a["median"]["analysis_s"]:.4f}/{b["median"]["analysis_s"]:.4f} '
               f'render_s={a["median"]["render_s"]:.4f}/{b["median"]["render_s"]:.4f} '
               f'encode_s={a["median"]["encode_s"]:.4f}/{b["median"]["encode_s"]:.4f}')
    print('::notice::' + summary)
(root / 'measurements.json').write_text(json.dumps(results, indent=2))
if not all(row['pcm_fingerprints_match'] for row in results.values()):
    raise SystemExit('Candidate did not match decoded PCM fingerprints; do not enable it.')
