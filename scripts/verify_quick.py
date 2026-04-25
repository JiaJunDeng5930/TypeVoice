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
    now_ms,
)


FIXTURE_LONG = os.path.join(REPO_ROOT, "fixtures", "zh_5m.ogg")


def main() -> int:
    if not os.path.exists(VENV_PYTHON):
        print("FAIL: .venv missing (create project-local venv first)")
        return 2

    ensure_fixtures_ready_or_exit(["zh_5m.ogg"])

    ensure_dirs()
    jsonl = os.path.join(REPO_ROOT, "metrics", "verify.jsonl")
    started_ms = now_ms()

    tauri_dir = os.path.join(REPO_ROOT, "apps", "desktop", "src-tauri")
    try:
        subprocess.check_call(["cargo", "check", "--locked"], cwd=tauri_dir)
    except Exception:
        print("FAIL: cargo check failed (backend compile gate)")
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
        return 1

    try:
        subprocess.check_call(
            [VENV_PYTHON, "-m", "pytest", "-q", "tests", "-m", "quick"],
            cwd=REPO_ROOT,
        )
    except Exception:
        print("FAIL: unit tests (quick) failed")
        return 1

    fail_reasons: list[str] = []
    cancel_ffmpeg_ms = cancel_ffmpeg_preprocess(
        FIXTURE_LONG,
        os.path.join(REPO_ROOT, "tmp", "quick_cancel.wav"),
        delay_ms=100,
    )
    if cancel_ffmpeg_ms > 300:
        fail_reasons.append(f"cancel_ffmpeg_slow:{cancel_ffmpeg_ms}ms")

    total_ms = now_ms() - started_ms
    status = "PASS" if not fail_reasons else "FAIL"
    print(f"{status}: cancel_ffmpeg_ms={cancel_ffmpeg_ms} total_ms={total_ms}")

    append_jsonl(
        jsonl,
        {
            "ts_ms": now_ms(),
            "level": "quick",
            "status": status,
            "cancel_ffmpeg_ms": cancel_ffmpeg_ms,
            "fail_reasons": fail_reasons,
        },
    )

    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
