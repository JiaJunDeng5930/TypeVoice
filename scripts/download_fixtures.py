#!/usr/bin/env python3
import os
import sys

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, REPO_ROOT)

from scripts._verify_util import ensure_fixtures_ready_or_exit


def main() -> int:
    ensure_fixtures_ready_or_exit(["zh_10s.ogg", "zh_60s.ogg", "zh_5m.ogg"])
    print("OK: fixtures ready")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
