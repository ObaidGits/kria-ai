"""
KRIA STT Sidecar — faster-whisper (CTranslate2) speech-to-text.

Voice System v3, Wave A. Replaces the in-process whisper-rs (CPU) path, which
measured 7-13 s/decode on the target hardware. faster-whisper `small` INT8 on
GPU measured ~0.23 s for the same clip while preserving Hinglish/English.

Design notes (see .kiro/specs/voice-system-v3/{requirements,design}.md):
- Default model `small`, compute `int8`, device auto (CUDA preferred, CPU
  fallback). Loaded once, kept warm (no per-turn reload).
- GPU/VRAM coordination (Wave A3): when device=auto and a CUDA GPU is present,
  CUDA is selected only if enough FREE VRAM exists for `small` INT8 (~460 MiB),
  else CPU INT8 — never OOM the resident LLM (Requirement 6.2, 6.6).
- Audio transport is BINARY (raw little-endian f32 PCM, 16 kHz mono) in the
  request body — never per-chunk JSON.
- The final transcript is the single authoritative result (Requirement 6.3).

Implemented with the Python standard library `http.server` so it needs no web
framework — only numpy + faster-whisper (CTranslate2).

Endpoints:
  GET  /health            -> liveness + which model/device actually loaded
  POST /transcribe        -> body: f32-LE PCM bytes; returns final transcript JSON
                             query: sample_rate, language, beam_size
"""

import json
import os
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

import numpy as np

try:
    from faster_whisper import WhisperModel
except Exception as _exc:  # pragma: no cover - import guard
    WhisperModel = None
    _IMPORT_ERROR = _exc
else:
    _IMPORT_ERROR = None


# ── Hinglish initial prompt ──
# Kept SHORT on purpose: faster-whisper echoes the initial_prompt verbatim when
# it decodes silence/noise (the source of the "Do not transliterate to
# Devanagari" hallucination). A short prompt still biases Latin-script Hinglish
# without giving the model a long phrase to regurgitate. Silence is additionally
# filtered by vad_filter + no_speech/echo guards below.
INITIAL_PROMPT = "Casual Hinglish conversation in Latin script."


def _norm(s: str) -> str:
    return " ".join(s.lower().split()).strip(" .!?,")


# Phrases Whisper commonly emits on silence/noise (prompt echoes + classic
# hallucinations). A final transcript equal to one of these is dropped.
_HALLUCINATIONS = {
    _norm(INITIAL_PROMPT),
    _norm("Do not transliterate to Devanagari"),
    _norm("Do not translate to Devanagari"),
    _norm("Preserve Latin spellings of Hindi words"),
    _norm("thank you"),
    _norm("thanks for watching"),
    _norm("thank you for watching"),
    _norm("please subscribe"),
    _norm("you"),
}

# Minimum free VRAM (bytes) before selecting CUDA for `small` INT8. The weights
# + compute buffers measured ~460 MiB; require headroom so the resident LLM is
# never OOM'd (Wave A3 VRAM coordination).
MIN_FREE_VRAM_BYTES = int(os.environ.get("KRIA_STT_MIN_FREE_VRAM", str(700 * 1024 * 1024)))


class SttEngine:
    """Holds a single warm faster-whisper model. Device/compute resolved once."""

    def __init__(self) -> None:
        self.requested_model = os.environ.get("KRIA_STT_MODEL", "small").strip() or "small"
        self.requested_device = os.environ.get("KRIA_STT_DEVICE", "auto").strip().lower() or "auto"
        self.requested_compute = os.environ.get("KRIA_STT_COMPUTE", "int8").strip() or "int8"
        self.model = None
        self.device = "cpu"
        self.compute_type = self.requested_compute
        self.load_error = ""
        # faster-whisper / CTranslate2 model is not guaranteed concurrency-safe;
        # serialize transcribe calls so overlapping partial+final requests from
        # the streaming client (Wave A2) cannot race the model.
        self._lock = threading.Lock()

    def _free_vram_bytes(self):
        """Best-effort free VRAM probe. Returns None if CUDA can't be queried."""
        try:
            import torch  # local import keeps CPU-only installs light

            if not torch.cuda.is_available():
                return None
            free, _total = torch.cuda.mem_get_info()
            return int(free)
        except Exception:
            return None

    def _resolve_device(self) -> str:
        if self.requested_device == "cpu":
            return "cpu"
        free = self._free_vram_bytes()
        if self.requested_device == "cuda":
            if free is not None and free < MIN_FREE_VRAM_BYTES:
                log(f"WARNING: cuda requested but only {free // (1024*1024)} MiB free; proceeding anyway")
            return "cuda"
        # auto: cuda only when present AND enough free VRAM for `small` INT8.
        if free is None:
            return "cpu"
        if free >= MIN_FREE_VRAM_BYTES:
            return "cuda"
        log(f"auto device: only {free // (1024*1024)} MiB free VRAM — using CPU to protect resident LLM")
        return "cpu"

    def load(self) -> None:
        if WhisperModel is None:
            self.load_error = f"faster-whisper import failed: {_IMPORT_ERROR}"
            log(self.load_error)
            return
        device = self._resolve_device()
        compute = self.requested_compute
        if device == "cuda" and compute == "int8":
            compute = "int8_float16"  # fast GPU compute type
        try:
            t0 = time.time()
            self.model = WhisperModel(self.requested_model, device=device, compute_type=compute)
            self.device = device
            self.compute_type = compute
            log(f"loaded model={self.requested_model} device={device} compute={compute} in {(time.time()-t0)*1000:.0f}ms")
        except Exception as exc:
            log(f"{device} load failed ({exc}); falling back to CPU int8")
            try:
                self.model = WhisperModel(self.requested_model, device="cpu", compute_type="int8")
                self.device = "cpu"
                self.compute_type = "int8"
                log(f"loaded model={self.requested_model} device=cpu compute=int8 (fallback)")
            except Exception as exc2:
                self.load_error = f"model load failed on cuda and cpu: {exc2}"
                log(self.load_error)

    def transcribe(self, audio: np.ndarray, language, beam_size: int):
        if self.model is None:
            raise RuntimeError(self.load_error or "model not loaded")
        duration_ms = int((len(audio) / 16000.0) * 1000.0)
        lang_arg = None if (not language or language.lower() == "auto") else language
        with self._lock:
            segments, info = self.model.transcribe(
                audio,
                language=lang_arg,
                beam_size=beam_size,
                initial_prompt=INITIAL_PROMPT,
                # Built-in Silero VAD: drop non-speech so the model never
                # decodes (and hallucinates on) silence/room noise.
                vad_filter=True,
                vad_parameters=dict(min_silence_duration_ms=500, speech_pad_ms=200),
                condition_on_previous_text=False,
                temperature=0.0,
                # Anti-hallucination thresholds: bail when a segment is likely
                # non-speech or low-confidence rather than emitting garbage.
                no_speech_threshold=0.6,
                log_prob_threshold=-1.0,
                compression_ratio_threshold=2.4,
            )
            parts = []
            logprobs = []
            no_speech = []
            for seg in segments:
                txt = seg.text.strip()
                if txt:
                    parts.append(txt)
                if seg.avg_logprob is not None:
                    logprobs.append(seg.avg_logprob)
                nsp = getattr(seg, "no_speech_prob", None)
                if nsp is not None:
                    no_speech.append(nsp)
        text = " ".join(parts).strip()
        if logprobs:
            confidence = float(np.clip(np.exp(np.mean(logprobs)), 0.0, 1.0))
        else:
            confidence = 0.0
        detected_lang = getattr(info, "language", None) or (lang_arg or "auto")

        # ── Hallucination / silence guards ───────────────────────────────
        norm = _norm(text)
        dropped = ""
        if not text:
            dropped = "empty"
        elif norm in _HALLUCINATIONS:
            dropped = "hallucination-phrase"
        elif norm and norm in _norm(INITIAL_PROMPT):
            dropped = "prompt-echo"
        elif no_speech and (sum(no_speech) / len(no_speech)) > 0.6 and len(norm) < 24:
            dropped = "high-no-speech"
        if dropped:
            log(f"dropped transcript ({dropped}): '{text[:60]}'")
            text = ""
            confidence = 0.0

        return {
            "text": text,
            "language": detected_lang,
            "confidence": confidence,
            "duration_ms": duration_ms,
            "engine": "faster-whisper",
            "device": self.device,
        }


def log(msg: str) -> None:
    print(f"[STT] {msg}", file=sys.stderr, flush=True)


_engine = SttEngine()


class Handler(BaseHTTPRequestHandler):
    # Silence the default per-request stderr logging (we log meaningfully above).
    def log_message(self, fmt, *args):  # noqa: N802
        return

    def _send_json(self, code: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # noqa: N802
        if urlparse(self.path).path != "/health":
            self._send_json(404, {"error": "not found"})
            return
        loaded = _engine.model is not None
        self._send_json(
            200,
            {
                "status": "healthy" if loaded else "degraded",
                "model": _engine.requested_model,
                "device": _engine.device,
                "compute_type": _engine.compute_type,
                "model_loaded": loaded,
                "detail": _engine.load_error,
            },
        )

    def do_POST(self):  # noqa: N802
        parsed = urlparse(self.path)
        if parsed.path != "/transcribe":
            self._send_json(404, {"error": "not found"})
            return
        qs = parse_qs(parsed.query)
        sample_rate = int(qs.get("sample_rate", ["16000"])[0])
        language = qs.get("language", [""])[0]
        beam_size = int(qs.get("beam_size", ["5"])[0])

        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length > 0 else b""
        if not raw:
            self._send_json(400, {"error": "empty audio body"})
            return
        if len(raw) % 4 != 0:
            self._send_json(400, {"error": "body length not a multiple of 4 (expected f32-LE PCM)"})
            return

        audio = np.frombuffer(raw, dtype="<f4").astype(np.float32)
        if sample_rate != 16000 and len(audio) > 0:
            target_len = int(len(audio) * 16000 / sample_rate)
            if target_len > 0:
                idx = np.linspace(0, len(audio) - 1, target_len)
                audio = np.interp(idx, np.arange(len(audio)), audio).astype(np.float32)

        try:
            t0 = time.time()
            resp = _engine.transcribe(audio, language, beam_size)
            log(
                f"transcribe: {resp['duration_ms']}ms audio -> '{resp['text'][:60]}' "
                f"({(time.time()-t0)*1000:.0f}ms, device={resp['device']}, lang={resp['language']})"
            )
            self._send_json(200, resp)
        except Exception as exc:
            log(f"transcribe failed: {exc}")
            self._send_json(503, {"error": str(exc)})


def main() -> None:
    _engine.load()
    host = os.environ.get("KRIA_STT_HOST", "127.0.0.1")
    port = int(os.environ.get("KRIA_STT_PORT", "8765"))
    server = ThreadingHTTPServer((host, port), Handler)
    log(f"listening on http://{host}:{port} (model={_engine.requested_model}, device={_engine.device})")
    # Signal readiness on stdout so a supervising parent can wait for it.
    print(f"READY http://{host}:{port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.shutdown()


if __name__ == "__main__":
    main()
