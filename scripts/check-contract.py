"""Pin the shared coordinator contract independently in each repository. Use --write after a deliberate protocol change."""

import argparse, hashlib, json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = [
    "control.rs",
    "coordination.rs",
    "constants.rs",
    "errors.rs",
    "model.rs",
    "opaque.rs",
    "presence.rs",
]


def contract_source(source):
    # Same rule as contract_source() in crates/core/src/lib.rs: comment lines, blank
    # lines and trailing spaces don't count, so editing comments never changes the contract.
    lines = (line.rstrip() for line in source.replace("\r\n", "\n").split("\n"))
    return "\n".join(
        line for line in lines if line.strip() and not line.lstrip().startswith("//")
    )


def fingerprint():
    source = "".join(
        (ROOT / "crates/core/src" / name).read_text(encoding="utf8") for name in FILES
    )
    return hashlib.sha256(contract_source(source).encode("utf8")).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    manifest = ROOT / "contract.json"
    actual = {"fingerprint": fingerprint(), "files": FILES}
    if args.write:
        manifest.write_text(json.dumps(actual, indent=2) + "\n", encoding="utf8")
    elif (
        not manifest.exists()
        or json.loads(manifest.read_text(encoding="utf8")) != actual
    ):
        raise SystemExit(
            "Shared contract changed. Review and port the matching core files to the other repository, then run scripts/check-contract.py --write in both."
        )
    print("Shared contract checked: " + actual["fingerprint"])


if __name__ == "__main__":
    main()
