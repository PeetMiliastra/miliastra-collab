import base64, importlib.util, unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "feed", Path(__file__).resolve().parents[1] / "scripts/make-update-feed.py"
)
feed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(feed)


def sig(version):
    return base64.b64encode(
        (
            "untrusted comment: fixture\nfixture\ntrusted comment: timestamp:1\tversion:"
            + version
            + "\nfixture"
        ).encode()
    ).decode()


class PortableFeed(unittest.TestCase):
    def test_portable_manifest_points_to_the_signed_executable(self):
        manifest = feed.make_manifest(
            "v0.1.33",
            "https://github.com/team/app/releases/download/v0.1.33/miliastra-collab.exe",
            sig("0.1.33"),
            "Notes",
        )
        self.assertEqual(manifest["format"], "portable-exe")
        self.assertEqual(manifest["version"], "0.1.33")
        self.assertEqual(
            manifest["platforms"]["windows-x86_64"]["signature"], sig("0.1.33")
        )

    def test_invalid_transport_missing_signatures_and_version_substitution_reject(self):
        for url, signature, version in [
            ("http://host/app.exe", sig("0.1.33"), "0.1.33"),
            ("https://host/app.zip", sig("0.1.33"), "0.1.33"),
            ("https://host/app.exe", "", "0.1.33"),
            ("https://host/app.exe", sig("0.1.32"), "0.1.33"),
            ("https://host/app.exe", sig("0.1.33"), "latest"),
        ]:
            with self.subTest(url=url, version=version):
                with self.assertRaises(ValueError):
                    feed.make_manifest(version, url, signature)


if __name__ == "__main__":
    unittest.main()
