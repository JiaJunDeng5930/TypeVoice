#!/usr/bin/env python3
import os
import sys


def main() -> int:
    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    venv_python = os.path.join(repo_root, ".venv", "bin", "python")
    if not os.path.exists(venv_python):
        print("FAIL: .venv missing")
        return 2

    # Run inside current interpreter (assumes .venv activated) if possible.
    try:
        from huggingface_hub import snapshot_download
    except Exception as e:
        print(f"FAIL: huggingface_hub not available in current env: {e}")
        print("Hint: activate .venv then retry")
        return 2

    repo_id = os.environ.get("TYPEVOICE_ASR_REPO", "Qwen/Qwen3-ASR-0.6B")
    out_dir = os.environ.get("TYPEVOICE_ASR_MODEL_DIR", os.path.join(repo_root, "models", "Qwen3-ASR-0.6B"))
    os.makedirs(out_dir, exist_ok=True)

    rev = snapshot_download(
        repo_id=repo_id,
        local_dir=out_dir,
        local_dir_use_symlinks=False,
        resume_download=True,
    )

    # Record the resolved revision for traceability.
    try:
        with open(os.path.join(out_dir, "REVISION.txt"), "w", encoding="utf-8") as f:
            f.write(str(rev) + "\n")
    except Exception:
        pass

    print(f"OK: downloaded {repo_id} -> {out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

