"""Native Windows release smoke. All state and helper processes belong to this run."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory
import time

exe = Path(sys.argv[1]).resolve() / "wrapped-collector.exe"


def run(env, *args, ok=True):
    result = subprocess.run([exe, *args], env=env, capture_output=True,
                            text=True, encoding="utf-8", timeout=15)
    if (result.returncode == 0) != ok:
        raise RuntimeError(f"{args}: {result.returncode}: {result.stdout} {result.stderr}")
    return result.stdout


def serve(env, children):
    child = subprocess.Popen([exe, "run"], env=env)
    children.append(child)
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError(f"collector exited early: {child.returncode}")
        ping = subprocess.run([exe, "ctl", "ping"], env=env, capture_output=True, timeout=10)
        if ping.returncode == 0 and json.loads(ping.stdout)["ok"]:
            return child
        time.sleep(0.1)
    raise TimeoutError("collector did not answer ping")


def status(env):
    return json.loads(run(env, "ctl", "status"))["data"]


def stop(env, child):
    assert json.loads(run(env, "ctl", "shutdown"))["ok"]
    assert child.wait(timeout=15) == 0


with TemporaryDirectory(prefix="Wrapped smoke ") as tmp:
    env = dict(os.environ, LOCALAPPDATA=tmp)
    env.pop("WRAPPED_DB", None)
    db = Path(tmp, "Wrapped", "wrapped.db")
    other = dict(env, WRAPPED_DB=str(db.with_suffix(".雪")))
    children = []
    try:
        # Load the packaged GUI executable and its DLL dependencies on MSVC.
        subprocess.run([exe.with_name("wrapped-dashboard.exe"), "--version"],
                       env=env, check=True, timeout=10, capture_output=True)
        for client in (env, other):
            run(client, "settings", "set", "onboarding.done", "true")
        assert db.is_file() and Path(other["WRAPPED_DB"]).is_file()
        first = serve(env, children)
        second = serve(other, children)
        run(env, "run", ok=False)
        assert json.loads(run(env, "ctl", "pause", '{"mode":"indefinite"}'))["ok"]
        assert status(env)["paused"] and not status(other)["paused"]
        run(env, "flush")
        stop(env, first)
        run(env, "ctl", "ping", ok=False)
        first = serve(env, children)
        assert status(env)["paused"]
        stop(env, first)
        second.kill()
        second.wait(timeout=10)
        run(other, "ctl", "ping", ok=False)
        stop(other, serve(other, children))
        for path in (db, Path(other["WRAPPED_DB"])):
            connection = sqlite3.connect(path)
            try:
                assert connection.execute("pragma quick_check").fetchone() == ("ok",)
            finally:
                connection.close()
        samples = [json.loads(line) for line in run(env, "now", "--count", "2", "--json").splitlines()]
        sample = samples[-1]
        assert 0 < sample["mem_used_bytes"] <= sample["mem_total_bytes"]
        assert sample["cpu_percent"] is not None and 0 <= sample["cpu_percent"] <= 100
        print("Runner Now sample:", json.dumps(sample, ensure_ascii=True))
        print("Writer exclusion, per-database commands, restart and shutdown passed.")
    finally:
        for child in children:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=10)
