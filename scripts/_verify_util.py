import hashlib
import json
import os
import shutil
import subprocess
import time
import urllib.request
from typing import Any


REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

def _venv_python_path(repo_root: str) -> str:
    if os.name == "nt":
        return os.path.join(repo_root, ".venv", "Scripts", "python.exe")
    return os.path.join(repo_root, ".venv", "bin", "python")


VENV_PYTHON = _venv_python_path(REPO_ROOT)

DEFAULT_FIXTURES_DIR = os.path.join(REPO_ROOT, "fixtures")
DEFAULT_FIXTURES_MANIFEST = os.path.join(REPO_ROOT, "scripts", "fixtures_manifest.json")


def _default_toolchain_dir() -> str:
    if os.name == "nt":
        return os.path.join(REPO_ROOT, "apps", "desktop", "src-tauri", "toolchain", "bin", "windows-x86_64")
    return os.path.join(REPO_ROOT, "apps", "desktop", "src-tauri", "toolchain", "bin", "linux-x86_64")


def resolve_tool_binary_or_fail(env_key: str, file_name: str) -> str:
    env_path = os.environ.get(env_key, "").strip()
    if env_path:
        if os.path.isfile(env_path):
            return env_path
        raise RuntimeError(f"E_TOOLCHAIN_NOT_READY: {env_key} points to missing file: {env_path}")

    toolchain_dir = os.environ.get("TYPEVOICE_TOOLCHAIN_DIR", "").strip() or _default_toolchain_dir()
    cand = os.path.join(toolchain_dir, file_name)
    if os.path.isfile(cand):
        return cand
    raise RuntimeError(
        f"E_TOOLCHAIN_NOT_READY: missing tool binary {cand} "
        f"(set {env_key} or TYPEVOICE_TOOLCHAIN_DIR)"
    )


def _sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _load_fixtures_manifest_or_exit() -> dict[str, Any]:
    manifest_path = os.environ.get("TYPEVOICE_FIXTURES_MANIFEST", "").strip() or DEFAULT_FIXTURES_MANIFEST
    if not os.path.exists(manifest_path):
        print(f"FAIL: fixtures manifest missing: {manifest_path}")
        raise SystemExit(2)
    try:
        with open(manifest_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception as e:
        print(f"FAIL: cannot parse fixtures manifest: {manifest_path} ({e})")
        raise SystemExit(2)

    fixtures = data.get("fixtures")
    if not isinstance(fixtures, list):
        print(f"FAIL: invalid fixtures manifest format: {manifest_path}")
        raise SystemExit(2)
    return data


def ensure_fixtures_ready_or_exit(required_files: list[str]) -> None:
    data = _load_fixtures_manifest_or_exit()
    fixtures = data["fixtures"]
    by_file: dict[str, dict[str, Any]] = {}
    for item in fixtures:
        if not isinstance(item, dict):
            continue
        name = item.get("file")
        if isinstance(name, str) and name:
            by_file[name] = item

    fixtures_dir = os.environ.get("TYPEVOICE_FIXTURES_DIR", "").strip() or DEFAULT_FIXTURES_DIR
    os.makedirs(fixtures_dir, exist_ok=True)

    for name in required_files:
        spec = by_file.get(name)
        if spec is None:
            print(f"FAIL: fixture not declared in manifest: {name}")
            raise SystemExit(2)

        url = str(spec.get("url") or "").strip()
        expected_sha256 = str(spec.get("sha256") or "").strip().lower()
        if not url or not expected_sha256:
            print(f"FAIL: fixture spec incomplete for: {name}")
            raise SystemExit(2)

        target_path = os.path.join(fixtures_dir, name)
        if os.path.exists(target_path):
            got = _sha256_file(target_path).lower()
            if got == expected_sha256:
                continue
            print(f"WARN: fixture checksum mismatch, re-downloading: {target_path}")

        tmp_path = target_path + ".download"
        try:
            with urllib.request.urlopen(url, timeout=120) as resp, open(tmp_path, "wb") as out:
                shutil.copyfileobj(resp, out)
        except Exception as e:
            if os.path.exists(tmp_path):
                os.remove(tmp_path)
            print(f"FAIL: fixture download failed for {name} from {url}: {e}")
            raise SystemExit(2)

        got_sha256 = _sha256_file(tmp_path).lower()
        if got_sha256 != expected_sha256:
            os.remove(tmp_path)
            print(f"FAIL: fixture checksum mismatch for {name}")
            print(f"  expected={expected_sha256}")
            print(f"  actual={got_sha256}")
            raise SystemExit(2)

        os.replace(tmp_path, target_path)
        print(f"INFO: fixture ready: {target_path}")


def now_ms() -> int:
    return int(time.time() * 1000)


def ensure_dirs() -> None:
    os.makedirs(os.path.join(REPO_ROOT, "metrics"), exist_ok=True)
    os.makedirs(os.path.join(REPO_ROOT, "tmp"), exist_ok=True)


def append_jsonl(path: str, obj: dict[str, Any]) -> None:
    with open(path, "a", encoding="utf-8") as f:
        f.write(json.dumps(obj, ensure_ascii=False) + "\n")


def ffprobe_duration_seconds(path: str) -> float:
    ffprobe = resolve_tool_binary_or_fail("TYPEVOICE_FFPROBE", "ffprobe.exe" if os.name == "nt" else "ffprobe")
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


def _ffmpeg_preprocess_args(
    input_path: str,
    output_path: str,
    silence_trim_enabled: bool = False,
    silence_threshold_db: float = -50.0,
    silence_start_ms: int = 300,
    silence_end_ms: int = 300,
) -> list[str]:
    args = [
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
        "-c:a",
        "pcm_s16le",
    ]
    if silence_trim_enabled:
        start = silence_start_ms / 1000.0
        end = silence_end_ms / 1000.0
        args.extend(
            [
                "-af",
                "silenceremove="
                f"start_periods=1:start_duration={start:.3f}:start_threshold={silence_threshold_db:.2f}dB"
                f":stop_periods=-1:stop_duration={end:.3f}:stop_threshold={silence_threshold_db:.2f}dB",
            ]
        )
    args.extend(["-vn", output_path])
    return args


def ffmpeg_preprocess_to_wav(input_path: str, output_path: str) -> int:
    """Convert audio to 16kHz mono PCM WAV. Returns elapsed ms."""
    ffmpeg = resolve_tool_binary_or_fail("TYPEVOICE_FFMPEG", "ffmpeg.exe" if os.name == "nt" else "ffmpeg")
    t0 = now_ms()
    subprocess.check_call([ffmpeg, *_ffmpeg_preprocess_args(input_path, output_path)])
    t1 = now_ms()
    return t1 - t0


def cancel_ffmpeg_preprocess(input_path: str, output_path: str, delay_ms: int = 100) -> int:
    """Start ffmpeg preprocess then kill; return cancel latency in ms."""
    ffmpeg = resolve_tool_binary_or_fail("TYPEVOICE_FFMPEG", "ffmpeg.exe" if os.name == "nt" else "ffmpeg")
    proc = subprocess.Popen(
        [ffmpeg, *_ffmpeg_preprocess_args(input_path, output_path)],
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
