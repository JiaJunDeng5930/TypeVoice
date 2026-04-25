#!/usr/bin/env python3
import os
import subprocess
import sys

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, REPO_ROOT)

from scripts._verify_util import (  # noqa: E402
    REPO_ROOT,
    VENV_PYTHON,
    append_jsonl,
    cancel_ffmpeg_preprocess,
    ensure_dirs,
    ensure_fixtures_ready_or_exit,
    ffmpeg_preprocess_to_wav,
    now_ms,
)


FIX_10 = os.path.join(REPO_ROOT, "fixtures", "zh_10s.ogg")
FIX_60 = os.path.join(REPO_ROOT, "fixtures", "zh_60s.ogg")
FIX_5M = os.path.join(REPO_ROOT, "fixtures", "zh_5m.ogg")


def main() -> int:
    if not os.path.exists(VENV_PYTHON):
        print("FAIL: .venv missing")
        return 2

    ensure_fixtures_ready_or_exit(["zh_10s.ogg", "zh_60s.ogg", "zh_5m.ogg"])

    ensure_dirs()
    jsonl = os.path.join(REPO_ROOT, "metrics", "verify.jsonl")
    started_ms = now_ms()

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

    try:
        subprocess.check_call(
            ["cargo", "test", "--locked", "concurrent_emit_keeps_jsonl_lines_parseable"],
            cwd=tauri_dir,
        )
        subprocess.check_call(
            ["cargo", "test", "--locked", "concurrent_metrics_emit_keeps_jsonl_lines_parseable"],
            cwd=tauri_dir,
        )
    except Exception:
        print("FAIL: debuggability contract tests failed")
        append_jsonl(
            jsonl,
            {
                "ts_ms": now_ms(),
                "level": "full",
                "status": "FAIL",
                "fail_reasons": ["debuggability_contract_tests_failed"],
            },
        )
        return 1

    try:
        subprocess.check_call([VENV_PYTHON, "-m", "pytest", "-q", "tests"], cwd=REPO_ROOT)
    except Exception:
        print("FAIL: unit tests failed")
        append_jsonl(jsonl, {"ts_ms": now_ms(), "level": "full", "status": "FAIL", "fail_reasons": ["unit_tests_failed"]})
        return 1

    tmp_dir = os.path.join(REPO_ROOT, "tmp", "preprocessed")
    os.makedirs(tmp_dir, exist_ok=True)
    out_10 = os.path.join(tmp_dir, "zh_10s.wav")
    out_60 = os.path.join(tmp_dir, "zh_60s.wav")
    out_5m = os.path.join(tmp_dir, "zh_5m.wav")

    fail_reasons: list[str] = []
    preprocess = {}
    try:
        preprocess["10s_ms"] = ffmpeg_preprocess_to_wav(FIX_10, out_10)
        preprocess["60s_ms"] = ffmpeg_preprocess_to_wav(FIX_60, out_60)
        preprocess["5m_ms"] = ffmpeg_preprocess_to_wav(FIX_5M, out_5m)
    except Exception as e:
        fail_reasons.append(f"preprocess_failed:{e}")

    cancel_ffmpeg_ms = cancel_ffmpeg_preprocess(FIX_5M, os.path.join(tmp_dir, "cancel.wav"), delay_ms=100)
    if cancel_ffmpeg_ms > 300:
        fail_reasons.append(f"cancel_ffmpeg_slow:{cancel_ffmpeg_ms}ms")

    status = "PASS" if not fail_reasons else "FAIL"
    total_ms = now_ms() - started_ms
    print(f"{status}: cancel_ffmpeg_ms={cancel_ffmpeg_ms} total_ms={total_ms}")

    append_jsonl(
        jsonl,
        {
            "ts_ms": now_ms(),
            "level": "full",
            "status": status,
            "preprocess": preprocess,
            "cancel_ffmpeg_ms": cancel_ffmpeg_ms,
            "fail_reasons": fail_reasons,
            "total_ms": total_ms,
        },
    )

    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
