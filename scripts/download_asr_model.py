#!/usr/bin/env python3
import hashlib
import json
import os
import sys


def _sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _walk_files(root_dir: str) -> list[dict]:
    out: list[dict] = []
    for base, _dirs, files in os.walk(root_dir):
        for name in files:
            full = os.path.join(base, name)
            rel = os.path.relpath(full, root_dir).replace("\\", "/")
            # Skip our own metadata files (we rewrite them).
            if rel in {"manifest.json", "REVISION.txt"}:
                continue
            st = os.stat(full)
            out.append(
                {
                    "path": rel,
                    "size": int(st.st_size),
                    "sha256": _sha256_file(full),
                }
            )
    out.sort(key=lambda x: x["path"])
    return out


def main() -> int:
    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    if os.name == "nt":
        venv_python = os.path.join(repo_root, ".venv", "Scripts", "python.exe")
    else:
        venv_python = os.path.join(repo_root, ".venv", "bin", "python")
    if not os.path.exists(venv_python):
        print("FAIL: .venv missing")
        return 2

    # Run inside current interpreter (assumes .venv activated) if possible.
    try:
        from huggingface_hub import HfApi, snapshot_download
    except Exception as e:
        print(f"FAIL: huggingface_hub not available in current env: {e}")
        print("Hint: activate .venv then retry")
        return 2

    repo_id = os.environ.get("TYPEVOICE_ASR_REPO", "Qwen/Qwen3-ASR-0.6B")
    out_dir = os.environ.get("TYPEVOICE_ASR_MODEL_DIR", os.path.join(repo_root, "models", "Qwen3-ASR-0.6B"))
    os.makedirs(out_dir, exist_ok=True)

    # Record a stable revision hash for traceability.
    api = HfApi()
    try:
        info = api.model_info(repo_id=repo_id)
        revision = getattr(info, "sha", None) or ""
    except Exception:
        revision = ""

    _ = snapshot_download(
        repo_id=repo_id,
        local_dir=out_dir,
        local_dir_use_symlinks=False,
        resume_download=True,
    )

    # Record revision and a local manifest for integrity checking.
    try:
        with open(os.path.join(out_dir, "REVISION.txt"), "w", encoding="utf-8") as f:
            f.write((revision or "").strip() + "\n")
            f.write(repo_id.strip() + "\n")
    except Exception:
        pass

    try:
        manifest = {
            "repo_id": repo_id,
            "revision": revision or None,
            "files": _walk_files(out_dir),
        }
        with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as f:
            json.dump(manifest, f, ensure_ascii=False, indent=2)
            f.write("\n")
    except Exception as e:
        print(f"WARN: failed to write manifest.json: {e}")

    print(f"OK: downloaded {repo_id} -> {out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
