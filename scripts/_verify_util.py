import json
import os
import signal
import subprocess
import time
from dataclasses import dataclass
from typing import Any


REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
VENV_PYTHON = os.path.join(REPO_ROOT, ".venv", "bin", "python")


def now_ms() -> int:
    return int(time.time() * 1000)


def ensure_dirs() -> None:
    os.makedirs(os.path.join(REPO_ROOT, "metrics"), exist_ok=True)
    os.makedirs(os.path.join(REPO_ROOT, "tmp"), exist_ok=True)


def append_jsonl(path: str, obj: dict[str, Any]) -> None:
    with open(path, "a", encoding="utf-8") as f:
        f.write(json.dumps(obj, ensure_ascii=False) + "\n")


def ffprobe_duration_seconds(path: str) -> float:
    out = subprocess.check_output(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path,
        ],
        text=True,
    ).strip()
    return float(out)


@dataclass(frozen=True)
class ProcResult:
    stdout_line: str
    elapsed_ms: int


def run_asr_once(model_id: str, audio_path: str, language: str = "Chinese") -> tuple[dict[str, Any], int]:
    """Start runner, send one request, read one response line, then exit."""
    proc = subprocess.Popen(
        [VENV_PYTHON, "-m", "asr_runner.runner", "--model", model_id],
        cwd=REPO_ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env={**os.environ, "PYTHONPATH": REPO_ROOT},
    )
    assert proc.stdin and proc.stdout
    t0 = now_ms()
    proc.stdin.write(json.dumps({"audio_path": audio_path, "language": language, "device": "cuda"}) + "\n")
    proc.stdin.flush()
    line = proc.stdout.readline().strip()
    t1 = now_ms()
    # Close stdin to encourage exit; then wait briefly.
    try:
        proc.stdin.close()
    except Exception:
        pass
    try:
        proc.wait(timeout=5)
    except Exception:
        proc.kill()
        proc.wait(timeout=5)
    return json.loads(line), (t1 - t0)


def cancel_asr_run(model_id: str, audio_path: str, delay_ms: int = 100) -> int:
    """Start runner, send request, then force-kill; return cancel latency in ms."""
    proc = subprocess.Popen(
        [VENV_PYTHON, "-m", "asr_runner.runner", "--model", model_id],
        cwd=REPO_ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
        env={**os.environ, "PYTHONPATH": REPO_ROOT},
    )
    assert proc.stdin
    proc.stdin.write(json.dumps({"audio_path": audio_path, "language": "Chinese", "device": "cuda"}) + "\n")
    proc.stdin.flush()
    t0 = now_ms()
    time.sleep(delay_ms / 1000.0)
    # SIGKILL to guarantee fast termination (we measure "cancel effective").
    proc.kill()
    proc.wait(timeout=5)
    t1 = now_ms()
    return t1 - t0

