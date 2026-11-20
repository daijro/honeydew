#!/usr/bin/env python3
import pathlib,re,statistics,json
root=pathlib.Path('benchmark-results')
rows=[dict(re.findall(r'(\w+)=([^ ]+)',line.split('CLIP_ENVELOPE_BENCH ',1)[1])) for line in (root/'envelopes.log').read_text().splitlines() if 'CLIP_ENVELOPE_BENCH ' in line]
assert len(rows)==18, len(rows)
report=['# Playlist envelope rendering workload','',
'Optimized Linux synthetic UI test: eight cached waveform meshes with/without both Playlist lanes; 64 egui frames and 64 CPU tessellations per repeat, median of three repeats. No Android FPS, GPU execution or whole-app CPU claim. Source mesh clone/translate and egui frame overhead included equally in both cases. Curve caches/fonts warmed before timing.','',
'| Controls per lane | Both lanes | Paint/frame ms | CPU tessellation ms | Vertices | Indices |','|---|---|---:|---:|---:|---:|']
for points in ['2','32','128']:
    messages=[]
    for enabled in ['false','true']:
        runs=[r for r in rows if r['points']==points and r['enabled']==enabled]
        assert len(runs)==3
        m={k:statistics.median(float(r[k]) for r in runs) for k in ['paint_ms','tess_ms','vertices','indices']}
        report.append(f'| {points} | {enabled} | {m["paint_ms"]:.4f} | {m["tess_ms"]:.4f} | {m["vertices"]:.0f} | {m["indices"]:.0f} |')
        messages.append(f'{enabled}: '+', '.join(f'{k}={v:.6f}' for k,v in m.items()))
    print('::notice title=Envelope rendering '+points+'::'+'; '.join(messages))
(root/'REPORT.md').write_text('\n'.join(report)+'\n')
(root/'measurements.json').write_text(json.dumps(rows,indent=2))
