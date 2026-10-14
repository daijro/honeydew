"""Make deterministic WAV inputs for isolated effect measurements."""

import argparse
import json
import math
import struct
import wave
from pathlib import Path


def _pcm16(value):
    return round(max(-1.0, min(1.0, value)) * 32767)


def _write_stereo(path, sample_rate, frames):
    with wave.open(str(path), "wb") as audio:
        audio.setnchannels(2)
        audio.setsampwidth(2)
        audio.setframerate(sample_rate)
        output = bytearray()
        for left, right in frames:
            output.extend(struct.pack("<hh", _pcm16(left), _pcm16(right)))
            if len(output) >= 32768:
                audio.writeframesraw(output)
                output.clear()
        audio.writeframes(output)


def generate_probes(directory: Path, sample_rate: int = 48000):
    if sample_rate not in (44100, 48000):
        raise ValueError("sample rate must be 44100 or 48000 Hz")
    directory.mkdir(parents=True, exist_ok=True)
    count = sample_rate * 4
    impulses = directory / f"impulses-{sample_rate}.wav"
    _write_stereo(
        impulses,
        sample_rate,
        ((0.25 if i == sample_rate // 4 else 0.0,
          0.25 if i == sample_rate * 2 else 0.0) for i in range(count)),
    )

    sweep = directory / f"sweep-{sample_rate}.wav"
    sweep_seconds = 3.0
    ratio = 18000.0 / 20.0
    log_ratio = math.log(ratio)

    def sweep_frames():
        for i in range(count):
            t = i / sample_rate
            if t >= sweep_seconds:
                yield 0.0, 0.0
                continue
            phase = 2 * math.pi * 20.0 * sweep_seconds * (
                math.exp(log_ratio * t / sweep_seconds) - 1.0
            ) / log_ratio
            fade = min(1.0, t / 0.05, (sweep_seconds - t) / 0.05)
            value = 0.25 * fade * math.sin(phase)
            yield value, value

    _write_stereo(sweep, sample_rate, sweep_frames())

    steps = directory / f"steps-{sample_rate}.wav"
    levels = (0.0, 0.05, 0.1, 0.2, 0.5, 0.8)

    def step_frames():
        for i in range(count):
            section = i // (sample_rate // 2)
            value = levels[section] if section < len(levels) else 0.0
            yield value, value * 0.25

    _write_stereo(steps, sample_rate, step_frames())

    transfer = directory / f"transfer-{sample_rate}.wav"

    def transfer_frames():
        for i in range(count):
            phase = (i / count) % 1.0
            triangle = 4.0 * abs(phase - 0.5) - 1.0
            value = 0.8 * triangle
            yield value, -value

    _write_stereo(transfer, sample_rate, transfer_frames())

    files = {
        "impulses": impulses,
        "sweep": sweep,
        "steps": steps,
        "transfer": transfer,
    }
    manifest = {
        "sample_rate": sample_rate,
        "format": "stereo PCM16 WAV",
        "duration_seconds": 4,
        "files": {key: path.name for key, path in files.items()},
        "impulses": {"left_seconds": 0.25, "right_seconds": 2.0, "amplitude": 0.25},
        "export": "Disable normalization and every effect except the one being measured. Keep channel and master levels fixed. Export one dry baseline with the same project settings.",
    }
    (directory / f"manifest-{sample_rate}.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    return files


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--sample-rate", type=int, default=48000)
    args = parser.parse_args()
    generate_probes(args.directory, args.sample_rate)
