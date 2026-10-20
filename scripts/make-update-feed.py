"""Build the portable-only manifest consumed by the desktop updater."""

import argparse, base64, json, re
from pathlib import Path
from urllib.parse import urlsplit


def make_manifest(version, url, signature, notes=""):
    version = version.removeprefix("v")
    if not re.fullmatch(
        r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?",
        version,
    ):
        raise ValueError("Use a semantic app version")
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or not parsed.path.lower().endswith(".exe")
    ):
        raise ValueError("Use the HTTPS download URL of the portable executable")
    signature = signature.strip()
    if not signature or len(signature) > 16384:
        raise ValueError("A portable app signature is required")
    text = base64.b64decode(signature, validate=True).decode("utf8")
    comment = next(
        (line for line in text.splitlines() if line.startswith("trusted comment: ")), ""
    )
    signed = next(
        (
            part[8:]
            for part in comment.removeprefix("trusted comment: ").split("\t")
            if part.startswith("version:")
        ),
        None,
    )
    if signed is None or signed.removeprefix("v") != version:
        raise ValueError("Sign the executable with --app-version matching this release")
    return {
        "version": version,
        "notes": notes,
        "format": "portable-exe",
        "platforms": {"windows-x86_64": {"url": url, "signature": signature}},
    }


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--version", required=True)
    p.add_argument("--url", required=True)
    p.add_argument("--signature", type=Path, required=True)
    p.add_argument("--notes", default="")
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    a.output.write_text(
        json.dumps(
            make_manifest(
                a.version, a.url, a.signature.read_text(encoding="utf8"), a.notes
            ),
            indent=2,
        )
        + "\n",
        encoding="utf8",
    )


if __name__ == "__main__":
    main()
