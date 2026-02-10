#!/usr/bin/env python3
import os
import sys
import subprocess

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, REPO_ROOT)

from scripts._verify_util import (  # noqa: E402
    REPO_ROOT,
    VENV_PYTHON,
    append_jsonl,
    cancel_asr_run,
    ensure_dirs,
    ffprobe_duration_seconds,
    now_ms,
    run_asr_once,
    resolve_asr_model_id_or_exit,
)


FIXTURE_10S = os.path.join(REPO_ROOT, "fixtures", "zh_10s.ogg")
FIXTURE_LONG = os.path.join(REPO_ROOT, "fixtures", "zh_5m.ogg")


def main() -> int:
    if not os.path.exists(VENV_PYTHON):
        print("FAIL: .venv missing (create project-local venv first)")
        return 2

    if not os.path.exists(FIXTURE_10S):
        print(f"FAIL: fixture missing: {FIXTURE_10S}")
        return 2

    ensure_dirs()
    jsonl = os.path.join(REPO_ROOT, "metrics", "verify.jsonl")
    started_ms = now_ms()
    model_id = resolve_asr_model_id_or_exit()

    # Compile gate (fast fail): make sure the Rust backend builds.
    tauri_dir = os.path.join(REPO_ROOT, "apps", "desktop", "src-tauri")
    try:
        subprocess.check_call(["cargo", "check", "--locked"], cwd=tauri_dir)
    except Exception:
        print("FAIL: cargo check failed (backend compile gate)")
        return 1

    # Unit tests (fast subset)
    try:
        subprocess.check_call(
            [VENV_PYTHON, "-m", "pytest", "-q", "tests", "-m", "quick"],
            cwd=REPO_ROOT,
        )
    except Exception:
        print("FAIL: unit tests (quick) failed")
        return 1

    # ASR smoke
    audio_seconds = ffprobe_duration_seconds(FIXTURE_10S)
    resp, wall_ms = run_asr_once(model_id=model_id, audio_path=FIXTURE_10S)

    ok = bool(resp.get("ok"))
    device_used = (resp.get("metrics") or {}).get("device_used") if isinstance(resp.get("metrics"), dict) else None
    rtf = (resp.get("metrics") or {}).get("rtf") if isinstance(resp.get("metrics"), dict) else None
    text = resp.get("text") if ok else None

    fail_reasons: list[str] = []
    if not ok:
        fail_reasons.append(f"asr_failed:{(resp.get('error') or {}).get('code')}")
    if not isinstance(text, str) or not text.strip():
        fail_reasons.append("empty_text")
    if device_used != "cuda":
        fail_reasons.append(f"device_not_cuda:{device_used}")
    if not isinstance(rtf, (int, float)):
        fail_reasons.append("missing_rtf")

    # Cancel smoke (force kill)
    cancel_latency_ms = cancel_asr_run(model_id=model_id, audio_path=FIXTURE_LONG, delay_ms=100)
    if cancel_latency_ms > 300:
        fail_reasons.append(f"cancel_slow:{cancel_latency_ms}ms")

    total_ms = now_ms() - started_ms
    status = "PASS" if not fail_reasons else "FAIL"
    print(f"{status}: rtf={rtf} device={device_used} cancel_ms={cancel_latency_ms} total_ms={total_ms}")

    append_jsonl(
        jsonl,
        {
            "ts_ms": now_ms(),
            "level": "quick",
            "status": status,
            "rtf": rtf,
            "device_used": device_used,
            "cancel_latency_ms": cancel_latency_ms,
            "wall_ms": wall_ms,
            "audio_seconds": audio_seconds,
            "fail_reasons": fail_reasons,
        },
    )

    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
