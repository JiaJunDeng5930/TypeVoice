#!/usr/bin/env python3
import argparse
import json
import os
import signal
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from typing import Any, Protocol


def _now_ms() -> int:
    return int(time.time() * 1000)


def _ffprobe_bin() -> str:
    p = os.environ.get("TYPEVOICE_FFPROBE", "").strip()
    return p or "ffprobe"


def _ffprobe_duration_seconds(path: str) -> float:
    # We rely on ffprobe being available in PATH during development. For the
    # desktop app we will bundle FFmpeg and point to it explicitly.
    out = subprocess.check_output(
        [
            _ffprobe_bin(),
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


class ProbePort(Protocol):
    def duration_seconds(self, path: str) -> float: ...


class DefaultProbePort:
    def duration_seconds(self, path: str) -> float:
        return _ffprobe_duration_seconds(path)


@dataclass(frozen=True)
class AsrError:
    code: str
    message: str
    details: dict[str, Any] | None = None


@dataclass(frozen=True)
class AsrMetrics:
    audio_seconds: float
    elapsed_ms: int
    rtf: float
    device_used: str
    model_id: str
    model_version: str | None = None


@dataclass(frozen=True)
class AsrSegment:
    index: int
    start_sec: float
    end_sec: float
    duration_sec: float
    text: str


@dataclass(frozen=True)
class AsrChunking:
    enabled: bool
    chunk_sec: float
    num_segments: int


@dataclass(frozen=True)
class AsrResponse:
    ok: bool
    text: str | None = None
    metrics: AsrMetrics | None = None
    error: AsrError | None = None
    segments: list[AsrSegment] | None = None
    chunking: AsrChunking | None = None


class _Terminated(Exception):
    pass


class RunnerRuntime:
    def __init__(self) -> None:
        self.exit_requested = False


def _install_signal_handlers(runtime: RunnerRuntime) -> None:
    def _handler(_signum: int, _frame: Any) -> None:
        runtime.exit_requested = True
        # We raise in the main thread to exit promptly from request loop.
        raise _Terminated()

    # SIGTERM is what our orchestrator/verify scripts will use for cancellation.
    signal.signal(signal.SIGTERM, _handler)


def _load_model(model_id: str, dtype: str, device_map: str, max_inference_batch_size: int) -> Any:
    import torch
    from qwen_asr import Qwen3ASRModel

    if not torch.cuda.is_available():
        raise RuntimeError("CUDA is not available (torch.cuda.is_available() is False).")

    if not device_map.startswith("cuda"):
        raise RuntimeError("CPU/device fallback is not allowed; device_map must be cuda.")

    if dtype == "float16":
        torch_dtype = torch.float16
    elif dtype == "bfloat16":
        torch_dtype = torch.bfloat16
    else:
        raise ValueError(f"Unsupported dtype: {dtype}")

    # Note: We intentionally keep config minimal for MVP. Decode params will be
    # frozen after perf spike.
    return Qwen3ASRModel.from_pretrained(
        model_id,
        torch_dtype=torch_dtype,
        device_map=device_map,
        max_inference_batch_size=max_inference_batch_size,
        max_new_tokens=4096,
    )


class ModelPort(Protocol):
    def transcribe(self, model: Any, audio: Any, language: str | None) -> Any: ...


class DefaultModelPort:
    def transcribe(self, model: Any, audio: Any, language: str | None) -> Any:
        return model.transcribe(audio=audio, language=language)


def _transcribe(model: Any, audio_path: str, language: str | None, model_port: ModelPort) -> str:
    # qwen-asr returns a list; each item has `.text` and `.language`.
    results = model_port.transcribe(model, audio_path, language)
    if not results:
        raise RuntimeError("Empty ASR result list.")
    text = getattr(results[0], "text", None)
    if not isinstance(text, str) or not text.strip():
        raise RuntimeError("Empty ASR text.")
    return text


def _handle_request(
    model: Any,
    model_id: str,
    chunk_sec: float,
    req: dict[str, Any],
    probe: ProbePort | None = None,
    model_port: ModelPort | None = None,
) -> AsrResponse:
    probe = probe or DefaultProbePort()
    model_port = model_port or DefaultModelPort()
    audio_path = req.get("audio_path")
    language = req.get("language", "Chinese")
    device = req.get("device", "cuda")

    if device != "cuda":
        return AsrResponse(
            ok=False,
            error=AsrError(code="E_DEVICE_NOT_ALLOWED", message="CPU/device fallback is not allowed."),
        )

    if not isinstance(audio_path, str) or not audio_path:
        return AsrResponse(
            ok=False,
            error=AsrError(code="E_BAD_REQUEST", message="audio_path is required."),
        )

    if not os.path.exists(audio_path):
        return AsrResponse(
            ok=False,
            error=AsrError(code="E_AUDIO_NOT_FOUND", message="audio_path does not exist.", details={"audio_path": audio_path}),
        )

    audio_seconds = probe.duration_seconds(audio_path)
    t0 = _now_ms()

    segments: list[AsrSegment] = []
    if audio_seconds > chunk_sec:
        # qwen-asr sets MAX_ASR_INPUT_SECONDS=1200 by default, so 5-minute audios
        # won't be split and may run close to realtime. We do an explicit chunking
        # pass here to keep long-audio performance within our RTF gates.
        from qwen_asr.inference.utils import SAMPLE_RATE, normalize_audios, split_audio_into_chunks

        wav = normalize_audios(audio_path)[0]
        parts = split_audio_into_chunks(wav=wav, sr=SAMPLE_RATE, max_chunk_sec=float(chunk_sec))
        chunk_audio = [(cwav, SAMPLE_RATE) for (cwav, _offset_sec) in parts]
        results = model_port.transcribe(model, chunk_audio, language)
        for i, (cwav, offset_sec) in enumerate(parts):
            t = getattr(results[i], "text", "") if i < len(results) else ""
            # cwav is a 1-D waveform (array-like)
            dur = float(len(cwav)) / float(SAMPLE_RATE) if SAMPLE_RATE else 0.0
            start = float(offset_sec)
            end = start + dur
            segments.append(
                AsrSegment(
                    index=i,
                    start_sec=start,
                    end_sec=end,
                    duration_sec=dur,
                    text=str(t or ""),
                )
            )
        text = "".join([s.text for s in segments if s.text is not None])
        if not text.strip():
            raise RuntimeError("Empty ASR text (chunked).")
    else:
        text = _transcribe(model, audio_path=audio_path, language=language, model_port=model_port)
        segments.append(
            AsrSegment(
                index=0,
                start_sec=0.0,
                end_sec=float(audio_seconds),
                duration_sec=float(audio_seconds),
                text=text,
            )
        )

    t1 = _now_ms()
    elapsed_ms = t1 - t0
    rtf = (elapsed_ms / 1000.0) / max(audio_seconds, 1e-6)

    metrics = AsrMetrics(
        audio_seconds=audio_seconds,
        elapsed_ms=elapsed_ms,
        rtf=rtf,
        device_used="cuda",
        model_id=model_id,
        model_version=_infer_model_version(model_id),
    )
    chunking = AsrChunking(
        enabled=bool(audio_seconds > chunk_sec),
        chunk_sec=float(chunk_sec),
        num_segments=len(segments),
    )
    return AsrResponse(ok=True, text=text, metrics=metrics, segments=segments, chunking=chunking)


def _infer_model_version(model_id: str) -> str | None:
    # If model_id is a local directory, try to read REVISION.txt written by our downloader.
    try:
        if os.path.isdir(model_id):
            p = os.path.join(model_id, "REVISION.txt")
            if os.path.exists(p):
                with open(p, "r", encoding="utf-8") as f:
                    line = (f.readline() or "").strip()
                    return line or None
    except Exception:
        return None
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description="TypeVoice ASR runner (Qwen3-ASR via qwen-asr).")
    parser.add_argument("--model", default="Qwen/Qwen3-ASR-0.6B")
    parser.add_argument("--dtype", default="float16", choices=["float16", "bfloat16"])
    parser.add_argument("--device-map", default="cuda:0")
    parser.add_argument("--max-inference-batch-size", type=int, default=8)
    parser.add_argument(
        "--chunk-sec",
        type=float,
        default=60.0,
        help="Split audio longer than this into smaller chunks (seconds) to improve throughput.",
    )
    parser.add_argument(
        "--protocol-only",
        action="store_true",
        help="Do not load model; only validate request/response protocol for unit tests.",
    )
    parser.add_argument(
        "--daemon",
        action="store_true",
        help="Load model once, emit a ready line, then handle multiple requests (JSONL).",
    )
    args = parser.parse_args()

    runtime = RunnerRuntime()
    _install_signal_handlers(runtime)
    probe = DefaultProbePort()
    model_port = DefaultModelPort()

    model = None
    t0 = _now_ms()
    if not args.protocol_only:
        # Load once, then handle multiple requests (JSONL).
        try:
            model = _load_model(
                model_id=args.model,
                dtype=args.dtype,
                device_map=args.device_map,
                max_inference_batch_size=args.max_inference_batch_size,
            )
        except Exception as e:
            resp = AsrResponse(ok=False, error=AsrError(code="E_MODEL_LOAD_FAILED", message=str(e)))
            sys.stdout.write(json.dumps(asdict(resp), ensure_ascii=False) + "\n")
            sys.stdout.flush()
            return 2
    t1 = _now_ms()

    if args.daemon:
        ready = {
            "type": "asr_ready",
            "ok": True,
            "model_id": args.model,
            "model_version": _infer_model_version(args.model),
            "device_used": "cuda",
            "warmup_ms": int(t1 - t0),
        }
        sys.stdout.write(json.dumps(ready, ensure_ascii=False) + "\n")
        sys.stdout.flush()

    while True:
        try:
            line = sys.stdin.readline()
            if not line:
                break
            if runtime.exit_requested:
                break
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except Exception as e:
                resp = AsrResponse(ok=False, error=AsrError(code="E_BAD_REQUEST", message=f"Invalid JSON: {e}"))
                sys.stdout.write(json.dumps(asdict(resp), ensure_ascii=False) + "\n")
                sys.stdout.flush()
                continue

            try:
                if args.protocol_only:
                    # Validate input shape and return a deterministic stub.
                    device = req.get("device", "cuda")
                    audio_path = req.get("audio_path")
                    if device != "cuda":
                        resp = AsrResponse(
                            ok=False,
                            error=AsrError(code="E_DEVICE_NOT_ALLOWED", message="CPU/device fallback is not allowed."),
                        )
                    elif not isinstance(audio_path, str) or not audio_path:
                        resp = AsrResponse(ok=False, error=AsrError(code="E_BAD_REQUEST", message="audio_path is required."))
                    else:
                        resp = AsrResponse(ok=False, error=AsrError(code="E_PROTOCOL_ONLY", message="protocol-only mode"))
                else:
                    assert model is not None
                    resp = _handle_request(
                        model,
                        model_id=args.model,
                        chunk_sec=args.chunk_sec,
                        req=req,
                        probe=probe,
                        model_port=model_port,
                    )
            except Exception as e:
                resp = AsrResponse(ok=False, error=AsrError(code="E_TRANSCRIBE_FAILED", message=str(e)))

            sys.stdout.write(json.dumps(asdict(resp), ensure_ascii=False) + "\n")
            sys.stdout.flush()
        except _Terminated:
            break
        except Exception as e:
            resp = AsrResponse(ok=False, error=AsrError(code="E_INTERNAL", message=str(e)))
            sys.stdout.write(json.dumps(asdict(resp), ensure_ascii=False) + "\n")
            sys.stdout.flush()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
