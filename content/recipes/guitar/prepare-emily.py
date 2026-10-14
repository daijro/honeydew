#!/usr/bin/env python3
"""Render Emilyguitar's layered SFZ voicings using installed NumPy and SciPy.

Run from the repository: python3 content/recipes/guitar/prepare-emily.py --sources ../content-sources
Writes only karoryfer-emilyguitar-rendered and the eight emily-*.toml voiced recipes.
Source recordings stay untouched. Run --check for the pitch and mapping self-check alone.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import wave

import numpy as np
import scipy
from scipy.signal import resample_poly

RATE = 44100
VELOCITIES = [(0, 40), (41, 80), (81, 120), (121, 127)]
PROGRAMS = [('three_voice', None, 'Three Voice'), ('five_voice', None, 'Five Voice')]
PROGRAMS += [(sfz, switch, label + wide) for sfz, wide in [('chords', ''), ('chords_wide', ' Wide')]
             for switch, label in [(33, 'Power Chords'), (34, 'Minor Chords'), (35, 'Major Chords')]]


def regions(path):
    levels = [{}, {}, {}, {}]
    text = '\n'.join(line.split('//')[0] for line in path.read_text().splitlines())
    parts = re.split(r'<(global|master|group|region)>', text)
    result = []
    for header, body in zip(parts[1::2], parts[2::2]):
        level = ['global', 'master', 'group', 'region'].index(header)
        levels[level:] = [{} for _ in range(4 - level)]
        levels[level] = dict(re.findall(r'(\w+)=([^\s<>]+)', body))
        if header == 'region':
            g = {k: v for d in levels for k, v in d.items()}
            if g.get('sample', '').startswith('notes') and float(g.get('lorand', 0)) <= 0 < float(g.get('hirand', 1)):
                result.append(g)
    return result


def active(groups, key, velocity, switch):
    return [g for g in groups if int(g.get('lokey', 0)) <= key <= int(g.get('hikey', 127))
            and int(g.get('lovel', 0)) <= velocity <= int(g.get('hivel', 127))
            and ('sw_last' not in g or int(g['sw_last']) == switch)]


def pitch(audio, semitones):
    rate = round(RATE * 2 ** (semitones / 12))
    divisor = math.gcd(rate, RATE)
    return resample_poly(audio, RATE // divisor, rate // divisor)


def write_wave(path, audio):
    # One fixed gain for every program, note and velocity keeps the recorded dynamic ratios.
    audio = audio / 8
    assert np.isfinite(audio).all() and np.max(np.abs(audio)) < 1, path
    pcm = np.rint(np.clip(audio, -1, 1 - 2 ** -23) * 2 ** 23).astype('<i4')
    packed = pcm.view('u1').reshape(-1, 4)[:, :3].tobytes()
    with wave.open(str(path), 'wb') as w:
        w.setparams((2, 3, RATE, len(audio), 'NONE', 'not compressed'))
        w.writeframes(packed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    source = args.sources / 'karoryfer-emilyguitar' / 'unpacked/karoryfer.emilyguitar-master'
    dest = args.sources / 'karoryfer-emilyguitar-rendered'
    specs = {name: regions(source / f'emily_{name}.sfz') for name, _, _ in PROGRAMS}
    expected = {33: {48, 55, 60}, 34: {48, 55, 60, 63, 67, 72}, 35: {48, 55, 60, 64, 67, 72}}
    notes = {'c': 0, 'db': 1, 'd': 2, 'eb': 3, 'e': 4, 'f': 5, 'gb': 6, 'g': 7, 'ab': 8, 'a': 9, 'bb': 10, 'b': 11}
    for switch, wanted in expected.items():
        pitches = set()
        for g in active(specs['chords'], 48, 80, switch):
            n, octave = re.match(r'notes\\([a-z]+)(\d)', g['sample']).groups()
            pitches.add(12 * (int(octave) + 1) + notes[n] + 48 - int(g.get('pitch_keycenter', 60)))
        assert pitches == wanted, (switch, pitches)
    assert len(active(specs['three_voice'], 48, 80, None)) == 3
    assert len(active(specs['five_voice'], 48, 80, None)) == 5
    tone = np.sin(2 * np.pi * 440 * np.arange(RATE) / RATE)
    raised = pitch(tone, 12)
    assert abs(np.fft.rfftfreq(len(raised), 1 / RATE)[np.argmax(abs(np.fft.rfft(raised)))] - 880) < 1
    print('Self-check passed: source chord pitches, voice counts, octave resampling.', flush=True)
    if args.check:
        return
    dest.mkdir(exist_ok=True)
    cache, inputs, outputs, programs = {}, {}, {}, []
    for name, switch, label in PROGRAMS:
        groups = specs[name]
        slug = label.lower().replace(' ', '-')
        rows = []
        for key in range(128):
            for layer, (low, high) in enumerate(VELOCITIES, 1):
                selected = active(groups, key, high, switch)
                if not selected:
                    continue
                pieces = []
                for g in selected:
                    path = source / g['sample'].replace('\\', '/')
                    if path not in cache:
                        data = path.read_bytes()
                        inputs[str(path.relative_to(args.sources))] = hashlib.sha256(data).hexdigest()
                        with wave.open(str(path)) as w:
                            assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 3
                            b = np.frombuffer(w.readframes(w.getnframes()), dtype='u1').reshape(-1, 3).astype('i4')
                        pcm = b[:, 0] | b[:, 1] << 8 | b[:, 2] << 16
                        cache[path] = ((pcm ^ (1 << 23)) - (1 << 23)) / 2 ** 23
                    mono = pitch(cache[path], key - int(g.get('pitch_keycenter', 60)))
                    pan = float(g.get('pan', 0)) / 100
                    assert -1 <= pan <= 1
                    angle = (pan + 1) * math.pi / 4
                    stereo = mono[:, None] * np.array([math.cos(angle), math.sin(angle)])
                    delay = round(float(g.get('delay', 0)) * RATE)
                    assert 0 <= delay <= RATE
                    pieces.append((delay, stereo))
                frames = max(delay + len(audio) for delay, audio in pieces)
                assert 0 < frames < RATE * 15, (label, key, frames)
                audio = np.zeros((frames, 2))
                for delay, piece in pieces:
                    audio[delay:delay + len(piece)] += piece
                filename = f'{slug}-{key}-v{layer}.wav'
                write_wave(dest / filename, audio)
                outputs[filename] = hashlib.sha256((dest / filename).read_bytes()).hexdigest()
                rows.append((filename, key, low, high))
        recipe = f'# Offline render of emily_{name}.sfz; original overlapping voices, pan, delay and velocity splits.\n# Preparation: content/recipes/guitar/prepare-emily.py. Fixed random choice; natural sample decay.\nid = "guitar-emily-{slug}"\nkind = "program"\npath = "Presets/DW Sampler/Guitar/Emilyguitar {label}.dwp"\nsource = "karoryfer-emilyguitar-rendered"\nchannels = "stereo"\ntail = {{ floor_db = -60, fade_ms = 30 }}\nrelease = 0.25\ngain = "normalize"\n'
        for file, key, low, high in rows:
            recipe += f'\n[[zone]]\nfile = "{file}"\nroot = {key}\nlovel = {low}\nhivel = {high}\n'
        (Path(__file__).parent / f'emily-{slug}.toml').write_text(recipe)
        programs.append(dict(name=label, zones=len(rows), keys=[min(r[1] for r in rows), max(r[1] for r in rows)]))
        print(label, len(rows), 'zones', flush=True)
    for name in specs:
        path = source / f'emily_{name}.sfz'
        inputs[str(path.relative_to(args.sources))] = hashlib.sha256(path.read_bytes()).hexdigest()
    provenance = dict(source='karoryfer-emilyguitar', license='CC0-1.0', script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), numpy=np.__version__, scipy=scipy.__version__, sample_rate=RATE, bits=24, channels=2, random_value=0, gain=0.125, inputs=inputs, outputs=outputs, programs=programs)
    (dest / 'PROVENANCE.json').write_text(json.dumps(provenance, indent=2) + '\n')


if __name__ == '__main__':
    main()
