import json
import os
import subprocess
import sys

import pytest


REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
VENV_PY = os.path.join(REPO_ROOT, ".venv", "bin", "python")


def _run_runner_protocol_only(payload: str) -> dict:
    env = {**os.environ, "PYTHONPATH": REPO_ROOT}
    p = subprocess.Popen(
        [VENV_PY, "-m", "asr_runner.runner", "--protocol-only"],
        cwd=REPO_ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        env=env,
    )
    assert p.stdin and p.stdout
    p.stdin.write(payload + "\n")
    p.stdin.flush()
    line = p.stdout.readline().strip()
    p.kill()
    p.wait(timeout=5)
    return json.loads(line)


@pytest.mark.quick
def test_runner_rejects_invalid_json():
    r = _run_runner_protocol_only("{not-json")
    assert r["ok"] is False
    assert r["error"]["code"] == "E_BAD_REQUEST"


@pytest.mark.quick
def test_runner_rejects_cpu_fallback():
    r = _run_runner_protocol_only(json.dumps({"audio_path": "x", "device": "cpu"}))
    assert r["ok"] is False
    assert r["error"]["code"] == "E_DEVICE_NOT_ALLOWED"


@pytest.mark.quick
def test_runner_requires_audio_path():
    r = _run_runner_protocol_only(json.dumps({"device": "cuda"}))
    assert r["ok"] is False
    assert r["error"]["code"] == "E_BAD_REQUEST"

