"""Focused Windows process/replacement tests. Only disposable executables are touched."""

import hashlib, json, os, shutil, stat, subprocess, time, uuid
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
root = Path(os.environ["MCOLLAB_PORTABLE_FIXTURE_ROOT"]).resolve()
assert root.parent == repo / ".tools" and root.name.startswith("portable-fixture-")
helper = Path(os.environ["MCOLLAB_PORTABLE_HELPER"]).resolve()
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
old = root / "old.exe"
new = root / "new.exe"
signature = (root / "new.exe.sig").read_text().strip()
run_root = root / "runs" / uuid.uuid4().hex
run_root.mkdir(parents=True)
current = json.loads((repo / "package.json").read_text())["version"]
parts = current.split(".")
future = ".".join(parts[:2] + [str(int(parts[2]) + 1)])
substitution = ".".join(parts[:2] + [str(int(parts[2]) + 2)])


def scenario(
    name,
    *,
    corrupt=False,
    version=future,
    fail_start=False,
    readonly=False,
    bad_original=False,
    profile=False,
):
    folder = run_root / name
    folder.mkdir()
    target = folder / "app.exe"
    shutil.copyfile(old, target)
    data = folder / "app-updates" / str(uuid.uuid4())
    data.mkdir(parents=True)
    shutil.copyfile(helper, data / "helper.exe")
    shutil.copyfile(new, data / "update.exe")
    if corrupt:
        b = bytearray((data / "update.exe").read_bytes())
        b[-1] ^= 1
        (data / "update.exe").write_bytes(b)
    if readonly:
        target.chmod(stat.S_IREAD)
    plan_path = data / "plan.json"
    parent = subprocess.Popen(
        [str(target), "--wait-for-update", str(plan_path)],
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    plan = dict(
        target=str(target),
        originalHash="0" * 64 if bad_original else digest(old),
        newHash=digest(data / "update.exe"),
        version=version,
        signature=signature,
        parent=parent.pid,
        args=["--profile", "B"] if profile else [],
        phase="prepared",
        error=None,
    )
    plan_path.write_text(json.dumps(plan), encoding="utf8")
    env = {**os.environ}
    if fail_start:
        env["MCOLLAB_FIXTURE_FAIL_LAUNCH"] = "1"
    try:
        with (data / "helper.log").open("w") as log:
            process = subprocess.Popen(
                [str(data / "helper.exe"), "--apply-portable-update", str(plan_path)],
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
            code = process.wait(timeout=45)
        result = json.loads(plan_path.read_text())
        success = (
            not any([corrupt, fail_start, readonly, bad_original]) and version == future
        )
        assert code == (0 if success else 2), (
            name,
            code,
            (data / "helper.log").read_text(),
        )
        if success:
            assert result["phase"] == "complete", result
            assert digest(target) == digest(new)
            assert digest(
                folder / (".miliastra-" + data.name + ".previous.exe")
            ) == digest(old)
            assert (data / "started").exists()
            if profile:
                assert (data / "relaunch-args.txt").read_text().splitlines()[:2] == [
                    "--profile",
                    "B",
                ]
            # Running the finished update again must not downgrade the app or start another update.
            replay = subprocess.run(
                [str(data / "helper.exe"), "--apply-portable-update", str(plan_path)],
                capture_output=True,
                creationflags=subprocess.CREATE_NO_WINDOW,
                timeout=10,
            )
            assert replay.returncode == 2 and digest(target) == digest(new)
            assert json.loads(plan_path.read_text())["phase"] == "complete"
        else:
            assert result["phase"] == "failed" and result["error"], result
            assert digest(target) == digest(old), (
                name,
                "Original app was not preserved",
            )
            if fail_start or readonly:
                for _ in range(100):
                    if (data / "rollback-relaunched").exists():
                        break
                    time.sleep(0.02)
                assert (data / "rollback-relaunched").exists(), (name, result)
            else:
                assert not (data / "ready").exists(), (
                    name,
                    "Invalid download must reject before the app closes",
                )
    finally:
        if parent.poll() is None:
            parent.terminate()
            parent.wait(timeout=5)
        if readonly:
            target.chmod(stat.S_IWRITE | stat.S_IREAD)
    print("Passed: " + name)


scenario("success", profile=True)
scenario("rollback", fail_start=True)
scenario("tampered", corrupt=True)
scenario("downgrade", version=current)
scenario("version-substitution", version=substitution)
scenario("changed-original", bad_original=True)
scenario("read-only", readonly=True)
print("Native portable helper checks passed; all files isolated under " + str(run_root))
