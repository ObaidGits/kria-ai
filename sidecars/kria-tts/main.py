"""
KRIA TTS Sidecar — Kokoro (ONNX, Apache-2.0) text-to-speech.

Voice System v3, Wave 5. Higher-quality, multilingual (incl. Hindi) TTS.
Piper remains the guaranteed fallback in the Rust pipeline; this sidecar is
selected only when `voice.tts_engine = "kokoro"` AND it is reachable/loaded.

Design notes (see .kiro/specs/voice-system-v3/{requirements,design}.md):
- Kokoro outputs 24 kHz mono float32 audio.
- Audio transport is BINARY (raw little-endian f32 PCM) in the response body.
- Loaded once, kept warm. CPU works; GPU used automatically if torch+CUDA.

Implemented with the Python standard library `http.server` (no web framework).
Requires the `kokoro` pip package + its model weights (downloaded on first
KPipeline construction). When unavailable, `/health` reports degraded and the
Rust client falls back to Piper.

Endpoints:
  GET  /health                 -> liveness + lang/voice + loaded flag
  POST /synthesize             -> body: UTF-8 text; query: voice, speed, lang
                                  returns raw f32-LE PCM (24 kHz mono)
"""

import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

import numpy as np

try:
    from kokoro import KPipeline
except Exception as _exc:  # pragma: no cover - import guard
    KPipeline = None
    _IMPORT_ERROR = _exc
else:
    _IMPORT_ERROR = None

SAMPLE_RATE = 24000


def log(msg: str) -> None:
    print(f"[TTS] {msg}", file=sys.stderr, flush=True)


class TtsEngine:
    def __init__(self) -> None:
        # lang_code: 'a' American English, 'b' British, 'h' Hindi, etc.
        self.lang_code = os.environ.get("KRIA_TTS_LANG", "a").strip() or "a"
        self.default_voice = os.environ.get("KRIA_TTS_VOICE", "af_heart").strip() or "af_heart"
        self.pipeline = None
        self.load_error = ""
        self._lock = threading.Lock()

    def load(self) -> None:
        if KPipeline is None:
            self.load_error = f"kokoro import failed: {_IMPORT_ERROR}"
            log(self.load_error)
            return
        try:
            self.pipeline = KPipeline(lang_code=self.lang_code)
            log(f"loaded Kokoro pipeline lang={self.lang_code} voice={self.default_voice}")
        except Exception as exc:
            self.load_error = f"KPipeline load failed: {exc}"
            log(self.load_error)

    def synthesize(self, text: str, voice: str, speed: float) -> np.ndarray:
        if self.pipeline is None:
            raise RuntimeError(self.load_error or "pipeline not loaded")
        with self._lock:
            chunks = []
            for _gs, _ps, audio in self.pipeline(text, voice=voice, speed=speed):
                arr = audio.detach().cpu().numpy() if hasattr(audio, "detach") else np.asarray(audio)
                chunks.append(arr.astype(np.float32).reshape(-1))
        if not chunks:
            return np.zeros(0, dtype=np.float32)
        return np.concatenate(chunks).astype("<f4")


_engine = TtsEngine()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):  # noqa: N802
        return

    def _send_json(self, code: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_pcm(self, pcm: np.ndarray) -> None:
        body = pcm.tobytes()
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("X-Sample-Rate", str(SAMPLE_RATE))
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # noqa: N802
        if urlparse(self.path).path != "/health":
            self._send_json(404, {"error": "not found"})
            return
        loaded = _engine.pipeline is not None
        self._send_json(
            200,
            {
                "status": "healthy" if loaded else "degraded",
                "engine": "kokoro",
                "lang": _engine.lang_code,
                "voice": _engine.default_voice,
                "sample_rate": SAMPLE_RATE,
                "model_loaded": loaded,
                "detail": _engine.load_error,
            },
        )

    def do_POST(self):  # noqa: N802
        parsed = urlparse(self.path)
        if parsed.path != "/synthesize":
            self._send_json(404, {"error": "not found"})
            return
        qs = parse_qs(parsed.query)
        voice = qs.get("voice", [_engine.default_voice])[0]
        speed = float(qs.get("speed", ["1.0"])[0])
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length > 0 else b""
        text = raw.decode("utf-8", "ignore").strip()
        if not text:
            self._send_json(400, {"error": "empty text"})
            return
        try:
            pcm = _engine.synthesize(text, voice, speed)
            self._send_pcm(pcm)
        except Exception as exc:
            log(f"synthesize failed: {exc}")
            self._send_json(503, {"error": str(exc)})


def main() -> None:
    _engine.load()
    host = os.environ.get("KRIA_TTS_HOST", "127.0.0.1")
    port = int(os.environ.get("KRIA_TTS_PORT", "8766"))
    server = ThreadingHTTPServer((host, port), Handler)
    log(f"listening on http://{host}:{port} (lang={_engine.lang_code}, loaded={_engine.pipeline is not None})")
    print(f"READY http://{host}:{port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.shutdown()


if __name__ == "__main__":
    main()
