import tempfile
import unittest
import wave
from pathlib import Path

from audio_probes import generate_probes


class AudioProbeTests(unittest.TestCase):
    def test_generates_importable_stereo_probes_with_known_timing(self):
        with tempfile.TemporaryDirectory() as directory:
            files = generate_probes(Path(directory), sample_rate=48000)
            self.assertEqual(set(files), {"impulses", "sweep", "steps", "transfer"})
            with wave.open(str(files["impulses"]), "rb") as audio:
                self.assertEqual(audio.getnchannels(), 2)
                self.assertEqual(audio.getframerate(), 48000)
                self.assertEqual(audio.getsampwidth(), 2)
                self.assertEqual(audio.getnframes(), 4 * 48000)
                audio.setpos(12000)
                self.assertNotEqual(audio.readframes(1), b"\x00\x00\x00\x00")
                audio.setpos(12001)
                self.assertEqual(audio.readframes(1), b"\x00\x00\x00\x00")


if __name__ == "__main__":
    unittest.main()
