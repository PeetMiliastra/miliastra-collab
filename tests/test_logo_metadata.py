"""Ensure distributed branding cannot carry author or design-tool metadata."""

import struct
import unittest
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# The app shows public/app-logo.png; the coordinator serves its core copy on invitation pages.
LOGO = ROOT / (
    "public/app-logo.png"
    if (ROOT / "src-tauri").is_dir()
    else "crates/core/assets/app-logo.png"
)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
RENDERING_CHUNKS = {b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"}


class LogoMetadataTests(unittest.TestCase):
    def assert_metadata_free_png(self, data):
        self.assertTrue(data.startswith(PNG_SIGNATURE))
        position = len(PNG_SIGNATURE)
        kinds = []
        image_data = []
        while position < len(data):
            self.assertGreaterEqual(len(data) - position, 12)
            length = struct.unpack_from(">I", data, position)[0]
            end = position + length + 12
            self.assertLessEqual(end, len(data))
            kind = data[position + 4 : position + 8]
            payload = data[position + 8 : end - 4]
            self.assertIn(
                kind, RENDERING_CHUNKS, "Logo contains non-rendering metadata"
            )
            self.assertEqual(
                zlib.crc32(kind + payload), struct.unpack_from(">I", data, end - 4)[0]
            )
            kinds.append(kind)
            if kind == b"IDAT":
                image_data.append(payload)
            position = end
            if kind == b"IEND":
                break
        self.assertEqual(position, len(data), "Trailing logo data is not allowed")
        self.assertEqual(kinds[0], b"IHDR")
        self.assertEqual(kinds[-1], b"IEND")
        self.assertTrue(zlib.decompress(b"".join(image_data)))

    def test_shipped_png_logos_have_only_rendering_chunks(self):
        paths = [LOGO]
        if (ROOT / "src-tauri").is_dir():
            paths.extend(sorted((ROOT / "src-tauri/icons").glob("*.png")))
            self.assertGreaterEqual(len(paths), 5)
        for path in paths:
            with self.subTest(asset=str(path.relative_to(ROOT))):
                self.assert_metadata_free_png(path.read_bytes())

    def test_logo_keeps_its_expected_dimensions(self):
        data = LOGO.read_bytes()
        self.assertEqual(data[12:16], b"IHDR")
        self.assertEqual(struct.unpack_from(">II", data, 16), (512, 512))

    def test_taskbar_icon_frames_have_no_metadata(self):
        path = ROOT / "src-tauri/icons/icon.ico"
        if not path.exists():
            self.skipTest("Coordinator does not distribute Windows icons")
        data = path.read_bytes()
        reserved, kind, count = struct.unpack_from("<HHH", data)
        self.assertEqual((reserved, kind), (0, 1))
        self.assertGreater(count, 0)
        expected_offset = 6 + count * 16
        for index in range(count):
            size, offset = struct.unpack_from("<II", data, 6 + index * 16 + 8)
            self.assertEqual(offset, expected_offset)
            frame = data[offset : offset + size]
            self.assertEqual(len(frame), size)
            with self.subTest(frame=index):
                self.assert_metadata_free_png(frame)
            expected_offset += size
        self.assertEqual(expected_offset, len(data))


if __name__ == "__main__":
    unittest.main()
