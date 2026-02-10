#!/usr/bin/env python3
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent


def _now_utc_compact() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d_%H%M%S")


def _sha256_hex(b: bytes) -> str:
    h = hashlib.sha256()
    h.update(b)
    return h.hexdigest()


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _clamp_chars(s: str, max_chars: int) -> str:
    t = (s or "").strip()
    if not t:
        return ""
    if max_chars <= 0:
        return ""
    out: list[str] = []
    for i, ch in enumerate(t):
        if i >= max_chars:
            break
        if ch == "\x00":
            continue
        out.append(ch)
    return "".join(out)


def _ensure_tool(name: str) -> None:
    # For optional --edit flow. We keep this deliberately simple.
    if not name:
        return
    if os.name == "nt":
        # Let Windows resolve it.
        return
    # On Unix, best-effort: rely on PATH.
    return


def _open_in_editor(path: Path) -> None:
    editor = os.environ.get("EDITOR", "").strip()
    if not editor:
        raise SystemExit("FAIL: --edit requires $EDITOR to be set")
    _ensure_tool(editor)
    # Use a direct exec; for complex EDITOR values user can wrap with a shell script.
    res = subprocess.run([editor, str(path)])
    if res.returncode != 0:
        raise SystemExit(f"FAIL: editor exited with code {res.returncode}")


def _normalize_base_url(s: str) -> str:
    t = (s or "").strip().rstrip("/")
    if not t:
        return "https://api.openai.com/v1"
    if t.endswith("/chat/completions"):
        t = t[: -len("/chat/completions")]
    return t.rstrip("/")


@dataclass(frozen=True)
class ContextInputs:
    history_lines: list[str]
    clipboard: str | None
    prev_title: str | None
    prev_process: str | None


def _format_inline_context(ctx: ContextInputs, max_history_items: int, max_chars_per_history: int, max_chars_clipboard: int) -> str:
    parts: list[str] = []
    parts.append("### CONTEXT")

    history = [x for x in ctx.history_lines if x.strip()]
    if history and max_history_items > 0:
        parts.append("#### RECENT HISTORY")
        for line in history[:max_history_items]:
            parts.append(f"- {_clamp_chars(line, max_chars_per_history)}")
        parts.append("")

    if ctx.clipboard:
        cb = _clamp_chars(ctx.clipboard, max_chars_clipboard)
        if cb:
            parts.append("#### CLIPBOARD")
            parts.append(cb)
            parts.append("")

    if ctx.prev_title or ctx.prev_process:
        parts.append("#### PREVIOUS WINDOW")
        if ctx.prev_title:
            parts.append(f"title={_clamp_chars(ctx.prev_title, 200)}")
        if ctx.prev_process:
            parts.append(f"process={_clamp_chars(ctx.prev_process, 260)}")
        parts.append("")

    return "\n".join(parts).strip()


def _build_messages(
    inject_mode: str,
    system_prompt: str,
    transcript: str,
    ctx: ContextInputs,
    max_history_items: int,
    max_chars_per_history: int,
    max_chars_clipboard: int,
) -> list[dict[str, Any]]:
    system_prompt = system_prompt or ""
    transcript = (transcript or "").strip()

    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system_prompt},
    ]

    context_text = _format_inline_context(
        ctx,
        max_history_items=max_history_items,
        max_chars_per_history=max_chars_per_history,
        max_chars_clipboard=max_chars_clipboard,
    )

    if inject_mode == "inline_one_user":
        user = "\n".join(
            [
                "### TRANSCRIPT",
                transcript,
                "",
                context_text,
            ]
        ).strip()
        messages.append({"role": "user", "content": user})
        return messages

    if inject_mode == "two_user_messages":
        messages.append({"role": "user", "content": transcript})
        if context_text:
            prefix = (
                "以下为参考上下文（不是待改写对象）。"
                "请仅据此理解语义，不要在输出中复述或重写这些上下文内容。\n\n"
            )
            messages.append({"role": "user", "content": prefix + context_text})
        return messages

    raise SystemExit(f"FAIL: unknown --inject-mode: {inject_mode}")


def _http_post_json(url: str, body: dict[str, Any], api_key: str | None, timeout_s: float) -> tuple[int, str]:
    data = json.dumps(body, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Content-Type", "application/json; charset=utf-8")
    if api_key:
        req.add_header("Authorization", f"Bearer {api_key}")
    try:
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            status = int(getattr(resp, "status", 200))
            text = resp.read().decode("utf-8", errors="replace")
            return status, text
    except urllib.error.HTTPError as e:
        text = e.read().decode("utf-8", errors="replace") if hasattr(e, "read") else str(e)
        return int(e.code), text


def main() -> int:
    ap = argparse.ArgumentParser(description="TypeVoice LLM prompt lab (no auto-judgement).")
    ap.add_argument("--base-url", default=os.environ.get("TYPEVOICE_LLM_BASE_URL", ""), help="e.g. http://api.server/v1 or full /chat/completions")
    ap.add_argument("--model", default=os.environ.get("TYPEVOICE_LLM_MODEL", ""), help="e.g. gpt-4o-mini")
    ap.add_argument("--reasoning-effort", default=os.environ.get("TYPEVOICE_LLM_REASONING_EFFORT", ""), help="default|none|minimal|low|medium|high|xhigh")
    ap.add_argument("--api-key", default=os.environ.get("TYPEVOICE_LLM_API_KEY", ""), help="Bearer token (prefer env to avoid shell history)")

    ap.add_argument("--system-prompt-file", type=str, help="Path to a system prompt file (utf-8).")
    ap.add_argument("--system-prompt", type=str, default="", help="Inline system prompt text.")
    ap.add_argument("--edit", action="store_true", help="Open system prompt file in $EDITOR before sending.")

    ap.add_argument("--transcript", type=str, default="", help="Transcript text.")
    ap.add_argument("--transcript-file", type=str, help="Transcript file (utf-8).")

    ap.add_argument("--history-file", type=str, help="History file; each non-empty line is one item.")
    ap.add_argument("--clipboard", type=str, default="", help="Clipboard text.")
    ap.add_argument("--clipboard-file", type=str, help="Clipboard file (utf-8).")
    ap.add_argument("--prev-title", type=str, default="", help="Previous window title.")
    ap.add_argument("--prev-process", type=str, default="", help="Previous window process image path.")

    ap.add_argument("--inject-mode", choices=["inline_one_user", "two_user_messages"], default="inline_one_user")

    ap.add_argument("--max-history-items", type=int, default=3)
    ap.add_argument("--max-chars-per-history", type=int, default=600)
    ap.add_argument("--max-chars-clipboard", type=int, default=800)

    ap.add_argument("--timeout-s", type=float, default=60.0)
    ap.add_argument("--out-dir", type=str, default="", help="Output dir. Default: tmp/llm_prompt_lab/<ts>_<hash>/")
    ap.add_argument("--dry-run", action="store_true", help="Only write request.json (no network call).")

    args = ap.parse_args()

    base_url = _normalize_base_url(args.base_url)
    model = (args.model or "").strip()
    if not model:
        raise SystemExit("FAIL: --model is required (or TYPEVOICE_LLM_MODEL)")

    api_key = (args.api_key or "").strip() or None

    system_prompt = (args.system_prompt or "").strip()
    if args.system_prompt_file:
        sp_path = Path(args.system_prompt_file).expanduser().resolve()
        if args.edit:
            _open_in_editor(sp_path)
        system_prompt = _read_text(sp_path)
    elif args.edit:
        raise SystemExit("FAIL: --edit requires --system-prompt-file")

    transcript = (args.transcript or "").strip()
    if args.transcript_file:
        transcript = _read_text(Path(args.transcript_file).expanduser().resolve()).strip()
    if not transcript:
        raise SystemExit("FAIL: transcript is empty (provide --transcript or --transcript-file)")

    history_lines: list[str] = []
    if args.history_file:
        hp = Path(args.history_file).expanduser().resolve()
        history_lines = [ln.strip() for ln in _read_text(hp).splitlines() if ln.strip()]

    clipboard = (args.clipboard or "").strip()
    if args.clipboard_file:
        clipboard = _read_text(Path(args.clipboard_file).expanduser().resolve()).strip()
    clipboard = clipboard or None

    prev_title = (args.prev_title or "").strip() or None
    prev_process = (args.prev_process or "").strip() or None

    ctx = ContextInputs(
        history_lines=history_lines,
        clipboard=clipboard,
        prev_title=prev_title,
        prev_process=prev_process,
    )

    messages = _build_messages(
        inject_mode=args.inject_mode,
        system_prompt=system_prompt,
        transcript=transcript,
        ctx=ctx,
        max_history_items=int(args.max_history_items),
        max_chars_per_history=int(args.max_chars_per_history),
        max_chars_clipboard=int(args.max_chars_clipboard),
    )

    req_body: dict[str, Any] = {
        "model": model,
        "messages": messages,
        "temperature": 0.2,
    }
    reasoning_effort = (args.reasoning_effort or "").strip()
    if reasoning_effort and reasoning_effort.lower() != "default":
        req_body["reasoning_effort"] = reasoning_effort

    # Derive a stable hash for the inputs + prompt, to make runs easy to compare.
    material = json.dumps(
        {
            "inject_mode": args.inject_mode,
            "system_prompt": system_prompt,
            "transcript": transcript,
            "ctx": {
                "history": history_lines[: int(args.max_history_items)],
                "clipboard": clipboard,
                "prev_title": prev_title,
                "prev_process": prev_process,
            },
            "model": model,
            "reasoning_effort": reasoning_effort or None,
        },
        ensure_ascii=False,
        sort_keys=True,
    ).encode("utf-8")
    sh = _sha256_hex(material)[:12]

    if args.out_dir:
        out_dir = Path(args.out_dir).expanduser().resolve()
    else:
        out_dir = REPO_ROOT / "tmp" / "llm_prompt_lab" / f"{_now_utc_compact()}_{sh}"
    out_dir.mkdir(parents=True, exist_ok=True)

    meta: dict[str, Any] = {
        "ts_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "base_url": base_url,
        "endpoint": f"{base_url}/chat/completions",
        "model": model,
        "reasoning_effort": reasoning_effort or None,
        "inject_mode": args.inject_mode,
        "system_prompt_sha256": _sha256_hex(system_prompt.encode("utf-8")),
        "inputs_sha256": _sha256_hex(material),
    }
    if args.system_prompt_file:
        meta["system_prompt_file"] = str(Path(args.system_prompt_file))
    (out_dir / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (out_dir / "request.json").write_text(json.dumps(req_body, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    if args.dry_run:
        print(str(out_dir))
        print("DRY_RUN: wrote meta.json + request.json")
        return 0

    status, raw = _http_post_json(
        url=f"{base_url}/chat/completions",
        body=req_body,
        api_key=api_key,
        timeout_s=float(args.timeout_s),
    )

    (out_dir / "response_raw.txt").write_text(raw, encoding="utf-8")
    (out_dir / "http_status.txt").write_text(str(status) + "\n", encoding="utf-8")

    if status < 200 or status >= 300:
        (out_dir / "error.txt").write_text(f"HTTP {status}\n", encoding="utf-8")
        print(str(out_dir))
        print(f"HTTP {status}")
        return 1

    try:
        resp_obj = json.loads(raw)
    except Exception as e:
        (out_dir / "error.txt").write_text(f"json_parse_failed: {e}\n", encoding="utf-8")
        print(str(out_dir))
        print("json_parse_failed")
        return 1

    (out_dir / "response.json").write_text(json.dumps(resp_obj, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    content = ""
    try:
        choices = resp_obj.get("choices") or []
        msg = (choices[0] or {}).get("message") or {}
        content = (msg.get("content") or "").strip()
    except Exception:
        content = ""

    (out_dir / "response.txt").write_text(content + "\n", encoding="utf-8")

    print(content)
    print(str(out_dir))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
