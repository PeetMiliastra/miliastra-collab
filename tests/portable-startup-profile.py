"""Prepare and inspect a fresh synthetic device database for startup acknowledgement."""

import hashlib, json, os, sqlite3, sys, uuid
from pathlib import Path

root = Path(os.environ["MCOLLAB_TEST_DATA_DIR"]).resolve()
assert root.name.startswith("portable-startup-")
if sys.argv[1] == "prepare":
    root.mkdir(exist_ok=True)
    (root / "coordination.json").write_text('{"serviceUrl":"http://127.0.0.1:19746"}')
    with sqlite3.connect(root / "collab.db") as db:
        db.execute("CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        settings = dict(
            endpoint="http://127.0.0.1:19746",
            saveRoot=str(root / "no-game-saves"),
            oauth=dict(microsoftClientId="", googleClientId=""),
            autostart=False,
            screenRecognition=False,
            autoApply=False,
        )
        db.executemany(
            "INSERT INTO kv VALUES(?,?)",
            [
                ("settings", json.dumps(settings)),
                (
                    "draft:synthetic",
                    json.dumps(
                        {
                            "summary": "Keep this private draft",
                            "details": "Stored before restart",
                        }
                    ),
                ),
            ],
        )
    exe = Path(os.environ["MCOLLAB_PORTABLE_HELPER"]).resolve()
    plan = root / "app-updates" / str(uuid.uuid4())
    plan.mkdir(parents=True)
    data = dict(
        target=str(exe),
        originalHash="0" * 64,
        newHash=hashlib.sha256(exe.read_bytes()).hexdigest(),
        version=json.loads(
            (Path(__file__).resolve().parents[1] / "package.json").read_text()
        )["version"],
        signature="verified-by-helper-in-separate-test",
        parent=0,
        args=["--headless-smoke"],
        phase="restarting",
        error=None,
    )
    (plan / "plan.json").write_text(json.dumps(data))
    (root / "journal-path.txt").write_text(str(plan / "plan.json"))
else:
    plan = Path((root / "journal-path.txt").read_text()).parent
    assert (plan / "started").read_bytes() == b"started"
    with sqlite3.connect(root / "collab.db") as db:
        draft = json.loads(
            db.execute("SELECT value FROM kv WHERE key='draft:synthetic'").fetchone()[0]
        )
        assert draft == dict(
            summary="Keep this private draft", details="Stored before restart"
        )
    print("Startup handshake and private draft preservation confirmed.")
