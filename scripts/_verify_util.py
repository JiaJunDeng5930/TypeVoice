import json
import os
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Any


REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

def _venv_python_path(repo_root: str) -> str:
    if os.name == "nt":
        return os.path.join(repo_root, ".venv", "Scripts", "python.exe")
    return os.path.join(repo_root, ".venv", "bin", "python")


VENV_PYTHON = _venv_python_path(REPO_ROOT)

DEFAULT_LOCAL_MODEL_DIR = os.path.join(REPO_ROOT, "models", "Qwen3-ASR-0.6B")


def resolve_asr_model_id_or_exit() -> str:
    """Resolve ASR model id/path without triggering implicit downloads."""
    m = os.environ.get("TYPEVOICE_ASR_MODEL", "").strip()
    if m:
        return m

    cfg = os.path.join(DEFAULT_LOCAL_MODEL_DIR, "config.json")
    if os.path.exists(cfg):
        return DEFAULT_LOCAL_MODEL_DIR

    print("FAIL: ASR model not found (offline default).")
    print(f"Expected local model dir: {DEFAULT_LOCAL_MODEL_DIR}")
    print("Hint: run: .venv/bin/python scripts/download_asr_model.py")
    raise SystemExit(2)


def now_ms() -> int:
    return int(time.time() * 1000)


def ensure_dirs() -> None:
    os.makedirs(os.path.join(REPO_ROOT, "metrics"), exist_ok=True)
    os.makedirs(os.path.join(REPO_ROOT, "tmp"), exist_ok=True)


def append_jsonl(path: str, obj: dict[str, Any]) -> None:
    with open(path, "a", encoding="utf-8") as f:
        f.write(json.dumps(obj, ensure_ascii=False) + "\n")


def ffprobe_duration_seconds(path: str) -> float:
    ffprobe = os.environ.get("TYPEVOICE_FFPROBE", "").strip() or "ffprobe"
    out = subprocess.check_output(
        [
            ffprobe,
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


def ffmpeg_preprocess_to_wav(input_path: str, output_path: str) -> int:
    """Convert audio to 16kHz mono PCM WAV. Returns elapsed ms."""
    ffmpeg = os.environ.get("TYPEVOICE_FFMPEG", "").strip() or "ffmpeg"
    t0 = now_ms()
    subprocess.check_call(
        [
            ffmpeg,
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            input_path,
            "-ac",
            "1",
            "-ar",
            "16000",
            "-vn",
            output_path,
        ]
    )
    t1 = now_ms()
    return t1 - t0


def cancel_ffmpeg_preprocess(input_path: str, output_path: str, delay_ms: int = 100) -> int:
    """Start ffmpeg preprocess then kill; return cancel latency in ms."""
    ffmpeg = os.environ.get("TYPEVOICE_FFMPEG", "").strip() or "ffmpeg"
    proc = subprocess.Popen(
        [
            ffmpeg,
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            input_path,
            "-ac",
            "1",
            "-ar",
            "16000",
            "-vn",
            output_path,
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    t0 = now_ms()
    time.sleep(delay_ms / 1000.0)
    proc.kill()
    proc.wait(timeout=5)
    t1 = now_ms()
    return t1 - t0


def run_asr_batch(model_id: str, requests: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Start runner once, send N requests, read N responses, then exit."""
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
    for req in requests:
        proc.stdin.write(json.dumps(req, ensure_ascii=False) + "\n")
    proc.stdin.flush()
    # Close stdin to tell runner we are done; it will drain and exit.
    proc.stdin.close()

    responses: list[dict[str, Any]] = []
    for _ in requests:
        line = proc.stdout.readline()
        if not line:
            break
        responses.append(json.loads(line))

    try:
        proc.wait(timeout=10)
    except Exception:
        proc.kill()
        proc.wait(timeout=5)
    return responses


def start_asr_runner(model_id: str) -> subprocess.Popen[str]:
    proc = subprocess.Popen(
        [VENV_PYTHON, "-m", "asr_runner.runner", "--model", model_id],
        cwd=REPO_ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        env={**os.environ, "PYTHONPATH": REPO_ROOT},
    )
    assert proc.stdin and proc.stdout
    return proc


def asr_roundtrip(proc: subprocess.Popen[str], req: dict[str, Any]) -> dict[str, Any]:
    assert proc.stdin and proc.stdout
    proc.stdin.write(json.dumps(req, ensure_ascii=False) + "\n")
    proc.stdin.flush()
    line = proc.stdout.readline()
    if not line:
        return {"ok": False, "error": {"code": "E_ASR_RUNNER_EOF", "message": "runner stdout EOF"}}
    return json.loads(line)


def stop_asr_runner(proc: subprocess.Popen[str]) -> None:
    try:
        if proc.stdin:
            proc.stdin.close()
    except Exception:
        pass
    try:
        proc.wait(timeout=5)
    except Exception:
        proc.kill()
        proc.wait(timeout=5)

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
