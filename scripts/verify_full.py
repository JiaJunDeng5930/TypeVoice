#!/usr/bin/env python3
import os
import sys
import time
import subprocess

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, REPO_ROOT)

from scripts._verify_util import (  # noqa: E402
    REPO_ROOT,
    VENV_PYTHON,
    append_jsonl,
    cancel_asr_run,
    cancel_ffmpeg_preprocess,
    ensure_dirs,
    ffmpeg_preprocess_to_wav,
    now_ms,
    run_asr_batch,
    start_asr_runner,
    asr_roundtrip,
    stop_asr_runner,
    resolve_asr_model_id_or_exit,
)


FIX_10 = os.path.join(REPO_ROOT, "fixtures", "zh_10s.ogg")
FIX_60 = os.path.join(REPO_ROOT, "fixtures", "zh_60s.ogg")
FIX_5M = os.path.join(REPO_ROOT, "fixtures", "zh_5m.ogg")


def _check_resp(label: str, resp: dict, max_rtf: float | None) -> list[str]:
    reasons: list[str] = []
    if not resp.get("ok"):
        reasons.append(f"{label}:asr_failed:{(resp.get('error') or {}).get('code')}")
        return reasons
    m = resp.get("metrics") or {}
    device = m.get("device_used")
    if device != "cuda":
        reasons.append(f"{label}:device_not_cuda:{device}")
    rtf = m.get("rtf")
    if not isinstance(rtf, (int, float)):
        reasons.append(f"{label}:missing_rtf")
    elif max_rtf is not None and rtf > max_rtf:
        reasons.append(f"{label}:rtf_too_high:{rtf}>{max_rtf}")
    text = resp.get("text")
    if not isinstance(text, str) or not text.strip():
        reasons.append(f"{label}:empty_text")
    return reasons


def main() -> int:
    if not os.path.exists(VENV_PYTHON):
        print("FAIL: .venv missing")
        return 2

    for p in [FIX_10, FIX_60, FIX_5M]:
        if not os.path.exists(p):
            print(f"FAIL: fixture missing: {p}")
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
        append_jsonl(
            jsonl,
            {"ts_ms": now_ms(), "level": "full", "status": "FAIL", "fail_reasons": ["cargo_check_failed"]},
        )
        return 1

    # Unit tests (full)
    try:
        subprocess.check_call([VENV_PYTHON, "-m", "pytest", "-q", "tests"], cwd=REPO_ROOT)
    except Exception:
        print("FAIL: unit tests failed")
        append_jsonl(jsonl, {"ts_ms": now_ms(), "level": "full", "status": "FAIL", "fail_reasons": ["unit_tests_failed"]})
        return 1

    # Preprocess fixtures -> wav (M2)
    tmp_dir = os.path.join(REPO_ROOT, "tmp", "preprocessed")
    os.makedirs(tmp_dir, exist_ok=True)
    out_10 = os.path.join(tmp_dir, "zh_10s.wav")
    out_60 = os.path.join(tmp_dir, "zh_60s.wav")
    out_5m = os.path.join(tmp_dir, "zh_5m.wav")

    preprocess = {}
    preprocess["10s_ms"] = ffmpeg_preprocess_to_wav(FIX_10, out_10)
    preprocess["60s_ms"] = ffmpeg_preprocess_to_wav(FIX_60, out_60)
    preprocess["5m_ms"] = ffmpeg_preprocess_to_wav(FIX_5M, out_5m)

    # ASR batch (single model load)
    reqs = [
        {"audio_path": out_10, "language": "Chinese", "device": "cuda"},
        {"audio_path": out_60, "language": "Chinese", "device": "cuda"},
        {"audio_path": out_5m, "language": "Chinese", "device": "cuda"},
    ]
    resps = run_asr_batch(model_id=model_id, requests=reqs)
    if len(resps) != 3:
        print("FAIL: asr batch returned insufficient responses")
        append_jsonl(
            jsonl,
            {"ts_ms": now_ms(), "level": "full", "status": "FAIL", "fail_reasons": ["asr_batch_short_read"]},
        )
        return 1

    # Thresholds from base spec (must)
    fail_reasons: list[str] = []
    fail_reasons += _check_resp("10s", resps[0], max_rtf=None)
    fail_reasons += _check_resp("60s", resps[1], max_rtf=0.30)
    fail_reasons += _check_resp("5m", resps[2], max_rtf=0.35)

    # Cancel coverage (ffmpeg + asr)
    cancel_ffmpeg_ms = cancel_ffmpeg_preprocess(FIX_5M, os.path.join(tmp_dir, "cancel.wav"), delay_ms=100)
    if cancel_ffmpeg_ms > 300:
        fail_reasons.append(f"cancel_ffmpeg_slow:{cancel_ffmpeg_ms}ms")

    cancel_asr_ms = cancel_asr_run(model_id=model_id, audio_path=out_5m, delay_ms=100)
    if cancel_asr_ms > 300:
        fail_reasons.append(f"cancel_asr_slow:{cancel_asr_ms}ms")

    # Timeboxed stability (3 minutes suggested/required by v0.1)
    soak_start = time.time()
    soak_runs = 0
    soak_fail_reason: str | None = None
    soak_proc = start_asr_runner(model_id)
    try:
        while (time.time() - soak_start) < 180:
            r = asr_roundtrip(soak_proc, {"audio_path": out_10, "language": "Chinese", "device": "cuda"})
            soak_runs += 1
            if not r.get("ok"):
                soak_fail_reason = f"asr_failed:{(r.get('error') or {}).get('code')}"
                break
            m = r.get("metrics") or {}
            if m.get("device_used") != "cuda":
                soak_fail_reason = f"device_not_cuda:{m.get('device_used')}"
                break
    finally:
        stop_asr_runner(soak_proc)

    if soak_fail_reason:
        fail_reasons.append(f"soak_failed:{soak_fail_reason}")

    status = "PASS" if not fail_reasons else "FAIL"
    total_ms = now_ms() - started_ms
    rtf_60 = (resps[1].get("metrics") or {}).get("rtf")
    rtf_5m = (resps[2].get("metrics") or {}).get("rtf")
    print(
        f"{status}: rtf_60={rtf_60} rtf_5m={rtf_5m} cancel_ffmpeg_ms={cancel_ffmpeg_ms} cancel_asr_ms={cancel_asr_ms} soak_runs={soak_runs} total_ms={total_ms}"
    )

    append_jsonl(
        jsonl,
        {
            "ts_ms": now_ms(),
            "level": "full",
            "status": status,
            "preprocess": preprocess,
            "rtf_60": rtf_60,
            "rtf_5m": rtf_5m,
            "cancel_ffmpeg_ms": cancel_ffmpeg_ms,
            "cancel_asr_ms": cancel_asr_ms,
            "soak_runs": soak_runs,
            "fail_reasons": fail_reasons,
            "total_ms": total_ms,
        },
    )

    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
