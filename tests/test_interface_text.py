"""Keep application-authored interface punctuation consistent."""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIDDLE_DOT = re.compile(
    chr(0x00B7) + r"|\\u(?:00b7|\{0*b7\})|&(?:middot|#0*183|#x0*b7);",
    re.IGNORECASE,
)


class InterfaceTextTests(unittest.TestCase):
    def test_application_sources_do_not_introduce_middle_dot_punctuation(self):
        files = {ROOT / "index.html"} if (ROOT / "index.html").exists() else set()
        for folder in ["src", "src-tauri/src", "crates/core/assets", "public"]:
            files.update(
                path
                for path in (ROOT / folder).rglob("*")
                if path.is_file()
                and path.suffix in {".ts", ".tsx", ".css", ".rs", ".html"}
            )
        self.assertTrue(files)
        violations = [
            str(path.relative_to(ROOT))
            for path in sorted(files)
            if MIDDLE_DOT.search(path.read_text(encoding="utf-8"))
        ]
        self.assertEqual(violations, [], "Use wording, commas, colons, or parentheses")

    def test_common_source_encodings_are_detected(self):
        for value in [
            chr(0x00B7),
            r"\u00b7",
            r"\u{b7}",
            "&middot;",
            "&#183;",
            "&#xB7;",
        ]:
            with self.subTest(encoding=ascii(value)):
                self.assertIsNotNone(MIDDLE_DOT.search(value))
        self.assertIsNone(
            MIDDLE_DOT.search("Objects: Prefabs (2), Characters: Classes")
        )


if __name__ == "__main__":
    unittest.main()
