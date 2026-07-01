"""
KRIA Vision Sidecar - OmniParser Python Service

FastAPI service for screen understanding with RFC 007 compliant output schema.
This is a scaffolding implementation with dummy model - real PyTorch/ONNX
model to be swapped in without breaking the API contract.
"""

import io
import time
import uuid
import os
import json
import base64
import urllib.request
import urllib.error
from dataclasses import dataclass, field
from typing import List, Optional, Tuple
from datetime import datetime

from fastapi import FastAPI, File, UploadFile, HTTPException
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field
from PIL import Image
import numpy as np

app = FastAPI(
    title="KRIA Vision Sidecar",
    description="OmniParser screen understanding service for KRIA GUI automation",
    version="0.1.0"
)


# ============================================================================
# RFC 007 Compliant Schema
# ============================================================================

class OmniElement(BaseModel):
    """Element detected on screen per RFC 007 Section 3.2."""
    id: str = Field(..., description="Unique element identifier")
    element_type: str = Field(..., description="Type: button, input, text, etc.")
    label: str = Field(..., description="Raw label text from OCR")
    label_wrapped: str = Field(..., description="Label wrapped in <evidence> tags")
    bbox: List[int] = Field(..., description="Bounding box [x1, y1, x2, y2]", min_length=4, max_length=4)
    confidence: float = Field(..., ge=0.0, le=1.0, description="Detection confidence")
    monitor_id: int = Field(default=0, description="Monitor ID for multi-display")
    dpi_scale: float = Field(default=1.0, description="DPI scaling factor")
    visual_hash: str = Field(..., description="Perceptual hash for verification")


class OmniParserOutput(BaseModel):
    """Full OmniParser output schema per RFC 007."""
    elements: List[OmniElement] = Field(..., description="Detected UI elements")
    screen_dimensions: List[int] = Field(..., description="Overall [width, height]", min_length=2, max_length=2)
    monitor_dimensions: List[List[int]] = Field(..., description="Per-monitor dimensions")
    timestamp: int = Field(..., description="Unix timestamp")
    visual_hash: str = Field(..., description="Full-screen visual hash")
    # Task 8 (Issue #1): honest model provenance + degraded flag so the Rust
    # consumer never treats a stub/unavailable result as authoritative.
    model: str = Field(default="", description="Model that produced these detections")
    degraded: bool = Field(default=False, description="True when no real model served (stub/unavailable) — elements are NOT authoritative")


class ParseResponse(BaseModel):
    """Response wrapper for parse endpoint."""
    success: bool
    data: OmniParserOutput
    processing_time_ms: float


class HealthResponse(BaseModel):
    """Health check response."""
    status: str
    version: str
    model_loaded: bool


# ============================================================================
# Dummy OmniParser Model (Scaffolding)
# ============================================================================

class DummyOmniParser:
    """
    Dummy model returning synthetic elements for API contract validation.
    
    This maintains the exact RFC 007 schema so the real model can be swapped
    in later without requiring Rust core changes.
    """
    
    def __init__(self):
        self.model_name = "dummy-omniparser-v0.1"
    
    def parse(self, image: Image.Image, monitor_id: int = 0) -> OmniParserOutput:
        """
        Parse screenshot and return synthetic elements.
        
        In production, this will:
        1. Run YOLO/ONNX detection model
        2. Run OCR on detected regions
        3. Calculate perceptual hashes
        4. Return structured output
        """
        width, height = image.size
        
        # Generate synthetic elements for testing
        elements = [
            OmniElement(
                id="txt_main",
                element_type="text",
                label="Main text area",
                label_wrapped="<evidence>Main text area</evidence>",
                bbox=[50, 50, 750, 500],
                confidence=0.95,
                monitor_id=monitor_id,
                dpi_scale=1.0,
                visual_hash=self._calculate_phash(image, [50, 50, 750, 500])
            ),
            OmniElement(
                id="btn_save",
                element_type="button",
                label="Save Document",
                label_wrapped="<evidence>Save Document</evidence>",
                bbox=[100, 200, 200, 250],
                confidence=0.95,
                monitor_id=monitor_id,
                dpi_scale=1.0,
                visual_hash=self._calculate_phash(image, [100, 200, 200, 250])
            ),
            OmniElement(
                id="btn_cancel",
                element_type="button",
                label="Cancel",
                label_wrapped="<evidence>Cancel</evidence>",
                bbox=[220, 200, 320, 250],
                confidence=0.92,
                monitor_id=monitor_id,
                dpi_scale=1.0,
                visual_hash=self._calculate_phash(image, [220, 200, 320, 250])
            ),
            OmniElement(
                id="input_filename",
                element_type="input",
                label="Filename input field",
                label_wrapped="<evidence>Filename input field</evidence>",
                bbox=[100, 100, 400, 140],
                confidence=0.88,
                monitor_id=monitor_id,
                dpi_scale=1.0,
                visual_hash=self._calculate_phash(image, [100, 100, 400, 140])
            ),
        ]
        
        # Calculate full-screen hash
        full_hash = self._calculate_fullscreen_hash(image)
        
        return OmniParserOutput(
            elements=elements,
            screen_dimensions=[width, height],
            monitor_dimensions=[[width, height]],
            timestamp=int(time.time()),
            visual_hash=full_hash,
            # Task 8: the dummy is a STUB — report honestly so the Rust consumer
            # degrades (vision_degraded) instead of treating these synthetic
            # boxes as real detections.
            model=self.model_name,
            degraded=True,
        )
    
    def _calculate_phash(self, image: Image.Image, bbox: List[int]) -> str:
        """Calculate perceptual hash for bbox region (scaffolding)."""
        # In production: extract region, resize to 32x32, DCT, extract top-left 8x8
        # For now: return mock hash based on bbox coordinates
        return f"phash_{bbox[0]}_{bbox[1]}_{bbox[2]}_{bbox[3]}"
    
    def _calculate_fullscreen_hash(self, image: Image.Image) -> str:
        """Calculate full-screen perceptual hash (scaffolding)."""
        # In production: pHash of downscaled full image
        return f"fullhash_{image.size[0]}_{image.size[1]}"


# Global model instance
_model: Optional[DummyOmniParser] = None


# ============================================================================
# Task 8 (Issue #1): Real VL-7B grounding model
# ============================================================================

class Vl7bOmniParser:
    """
    Real visual grounding via a locally-served Qwen2.5-VL-7B-Instruct
    `llama-server` (OpenAI-compatible `/v1/chat/completions`, multimodal).

    Sends a DOWNSCALED screenshot + a grounding instruction and parses a JSON
    array of `{label, type, bbox}` detections. On ANY failure (server down,
    OOM, malformed output) it returns an HONEST degraded result with NO
    elements — it never fabricates detections (Requirement 1.2).

    Selected only when `KRIA_VISION_MODEL=vl7b`; otherwise the dummy stub is
    used (which reports `degraded=True`), so the contract is unchanged.
    """

    def __init__(self):
        self.model_name = "qwen3-vl-4b"
        self.endpoint = os.environ.get(
            "KRIA_VL_ENDPOINT", "http://127.0.0.1:8090/v1/chat/completions"
        )
        # Longest-side downscale target (keeps grounding cheap + legible).
        self.max_side = int(os.environ.get("KRIA_VL_MAX_SIDE", "1280"))

    def parse(self, image: Image.Image, monitor_id: int = 0) -> OmniParserOutput:
        width, height = image.size
        full_hash = f"fullhash_{width}_{height}"
        try:
            elements = self._ground(image, monitor_id)
            return OmniParserOutput(
                elements=elements,
                screen_dimensions=[width, height],
                monitor_dimensions=[[width, height]],
                timestamp=int(time.time()),
                visual_hash=full_hash,
                model=self.model_name,
                degraded=False,
            )
        except Exception as exc:  # honest degrade — NEVER fabricate detections
            print(f"[VISION] VL-7B grounding degraded: {exc}")
            return OmniParserOutput(
                elements=[],
                screen_dimensions=[width, height],
                monitor_dimensions=[[width, height]],
                timestamp=int(time.time()),
                visual_hash=full_hash,
                model=self.model_name,
                degraded=True,
            )

    def _ground(self, image: Image.Image, monitor_id: int) -> List[OmniElement]:
        downscaled = self._downscale(image)
        buf = io.BytesIO()
        downscaled.save(buf, format="PNG")
        b64 = base64.b64encode(buf.getvalue()).decode("ascii")
        prompt = (
            "You are a GUI grounding model. Return ONLY a compact JSON array of "
            "the interactive on-screen controls you can see, each as "
            '{"label": str, "type": one of [button,text,input,link,checkbox,tab,menu,dialog], '
            '"bbox": [x1,y1,x2,y2]} in pixels of the provided image. No prose.'
        )
        payload = {
            "model": self.model_name,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": prompt},
                        {
                            "type": "image_url",
                            "image_url": {"url": f"data:image/png;base64,{b64}"},
                        },
                    ],
                }
            ],
            "temperature": 0.0,
            "max_tokens": 1024,
        }
        req = urllib.request.Request(
            self.endpoint,
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=float(os.environ.get("KRIA_VL_TIMEOUT", "20"))) as resp:
            body = json.loads(resp.read().decode("utf-8"))
        content = body["choices"][0]["message"]["content"]
        # Scale detections back to the ORIGINAL image space.
        sx = image.size[0] / max(1, downscaled.size[0])
        sy = image.size[1] / max(1, downscaled.size[1])
        raw = json.loads(self._extract_json_array(content))
        elements: List[OmniElement] = []
        for i, item in enumerate(raw):
            bbox = item.get("bbox") or []
            if len(bbox) != 4:
                continue
            x1, y1, x2, y2 = (int(bbox[0] * sx), int(bbox[1] * sy), int(bbox[2] * sx), int(bbox[3] * sy))
            label = str(item.get("label", ""))[:200]
            elements.append(
                OmniElement(
                    id=f"vl7b_{i}",
                    element_type=str(item.get("type", "text")),
                    label=label,
                    label_wrapped=f"<evidence>{label}</evidence>",
                    bbox=[x1, y1, x2, y2],
                    confidence=0.9,
                    monitor_id=monitor_id,
                    dpi_scale=1.0,
                    visual_hash=f"phash_{x1}_{y1}_{x2}_{y2}",
                )
            )
        return elements

    def _downscale(self, image: Image.Image) -> Image.Image:
        w, h = image.size
        longest = max(w, h)
        if longest <= self.max_side:
            return image
        scale = self.max_side / longest
        return image.resize((max(1, int(w * scale)), max(1, int(h * scale))))

    @staticmethod
    def _extract_json_array(text: str) -> str:
        start = text.find("[")
        end = text.rfind("]")
        if start == -1 or end == -1 or end <= start:
            return "[]"
        return text[start : end + 1]


# ============================================================================
# Real OmniParser backend (lightweight): YOLO icon detector + optional caption.
#
# Selected with `KRIA_VISION_MODEL=omniparser`. Weights are loaded lazily from
# `KRIA_OMNIPARSER_WEIGHTS` (an ultralytics-compatible .pt icon detector). When
# ultralytics / the weights are unavailable, `parse` returns an HONEST degraded
# result (no fabricated elements) so the V2 Sight layer degrades cleanly. An
# optional caption model (`KRIA_OMNIPARSER_CAPTION`, a Florence-2 dir via
# transformers) labels each detected region; without it, the detector class name
# is used as the label.
# ============================================================================

class OmniParser:
    def __init__(self):
        self.model_name = "omniparser"
        self.weights = os.environ.get("KRIA_OMNIPARSER_WEIGHTS", "").strip()
        self.caption_dir = os.environ.get("KRIA_OMNIPARSER_CAPTION", "").strip()
        self._detector = None
        self._caption = None
        self._caption_proc = None
        self._load_error: Optional[str] = None
        # Task 6: fast OCR labelling (default ON). Florence-2 captioning is
        # accurate but far too slow on CPU for the many boxes a desktop screen
        # yields, so by default each detected box is labelled from OCR text that
        # falls inside it, and un-boxed OCR lines are surfaced as `text` elements
        # so "describe the screen" / text verification works for ANY app.
        import shutil as _shutil

        self.ocr_enabled = os.environ.get("KRIA_OMNIPARSER_OCR", "1") != "0"
        self._tesseract = _shutil.which("tesseract")
        self.detect_conf = float(os.environ.get("KRIA_OMNIPARSER_CONF", "0.05"))
        self.max_elements = int(os.environ.get("KRIA_OMNIPARSER_MAX_ELEMENTS", "200"))

    def _ensure_loaded(self):
        if self._detector is not None or self._load_error is not None:
            return
        try:
            from ultralytics import YOLO  # type: ignore

            if not self.weights or not os.path.exists(self.weights):
                raise RuntimeError(
                    f"icon-detector weights not found (KRIA_OMNIPARSER_WEIGHTS={self.weights!r})"
                )
            self._detector = YOLO(self.weights)
        except Exception as exc:
            self._load_error = f"omniparser detector unavailable: {exc}"
            return
        # Caption model is optional; failure here is non-fatal. OmniParser ships
        # the caption as fine-tuned WEIGHTS only — the processor must come from
        # the base Florence-2 repo (the local dir has no processor config).
        if self.caption_dir:
            try:
                from transformers import AutoModelForCausalLM, AutoProcessor  # type: ignore

                base = os.environ.get(
                    "KRIA_OMNIPARSER_CAPTION_BASE", "microsoft/Florence-2-base-ft"
                )
                self._caption = AutoModelForCausalLM.from_pretrained(
                    self.caption_dir, trust_remote_code=True
                )
                self._caption_proc = AutoProcessor.from_pretrained(
                    base, trust_remote_code=True
                )
            except Exception as exc:
                print(f"[VISION] OmniParser caption disabled (non-fatal): {exc}")

    def parse(self, image: Image.Image, monitor_id: int = 0) -> OmniParserOutput:
        width, height = image.size
        full_hash = f"fullhash_{width}_{height}"
        self._ensure_loaded()
        if self._load_error is not None:
            print(f"[VISION] OmniParser degraded: {self._load_error}")
            return OmniParserOutput(
                elements=[],
                screen_dimensions=[width, height],
                monitor_dimensions=[[width, height]],
                timestamp=int(time.time()),
                visual_hash=full_hash,
                model=self.model_name,
                degraded=True,
            )
        try:
            elements = self._detect(image, monitor_id)
            return OmniParserOutput(
                elements=elements,
                screen_dimensions=[width, height],
                monitor_dimensions=[[width, height]],
                timestamp=int(time.time()),
                visual_hash=full_hash,
                model=self.model_name,
                degraded=False,
            )
        except Exception as exc:
            print(f"[VISION] OmniParser detect degraded: {exc}")
            return OmniParserOutput(
                elements=[],
                screen_dimensions=[width, height],
                monitor_dimensions=[[width, height]],
                timestamp=int(time.time()),
                visual_hash=full_hash,
                model=self.model_name,
                degraded=True,
            )

    def _detect(self, image: Image.Image, monitor_id: int) -> List[OmniElement]:
        results = self._detector.predict(  # type: ignore
            image, conf=self.detect_conf, verbose=False
        )
        # Run OCR ONCE over the whole frame (fast on CPU); reuse for every box
        # label and for surfacing un-boxed text regions. Caption (if loaded) wins
        # over OCR for a box, since it describes icons OCR can't read.
        ocr_words = self._ocr_words(image) if self.ocr_enabled else []

        boxed: List[Tuple[List[int], str, str, float]] = []  # (xyxy, kind, label, conf)
        for result in results:
            boxes = getattr(result, "boxes", None)
            if boxes is None:
                continue
            names = getattr(result, "names", {}) or {}
            for box in boxes:
                xyxy = [int(v) for v in box.xyxy[0].tolist()]
                conf = float(box.conf[0].item()) if box.conf is not None else 0.0
                cls_id = int(box.cls[0].item()) if box.cls is not None else -1
                cls_name = names.get(cls_id, "icon")
                label = (
                    self._caption_region(image, xyxy)
                    or self._label_from_ocr(xyxy, ocr_words)
                    or cls_name
                )
                boxed.append((xyxy, cls_name, label, conf))

        elements: List[OmniElement] = []
        idx = 0
        for xyxy, cls_name, label, conf in boxed:
            elements.append(self._mk_element(idx, cls_name, label, xyxy, conf, monitor_id))
            idx += 1

        # Surface OCR text LINES that don't already sit inside a detected box, as
        # read-only `text` elements. This makes the screen describable/verifiable
        # for apps the icon detector under-covers (canvas/Electron/unseen apps),
        # WITHOUT fabricating interactivity (interactable=False for these).
        for line_text, lb in self._ocr_lines(ocr_words):
            if not line_text.strip():
                continue
            if any(self._overlaps(lb, b[0]) for b in boxed):
                continue
            elements.append(self._mk_element(idx, "text", line_text, lb, 0.5, monitor_id))
            idx += 1
            if idx >= self.max_elements:
                break

        return elements[: self.max_elements]

    def _mk_element(
        self, idx: int, cls_name: str, label: str, xyxy: List[int], conf: float, monitor_id: int
    ) -> OmniElement:
        label = (label or cls_name)[:200]
        return OmniElement(
            id=f"omni_{idx}",
            element_type=cls_name,
            label=label,
            label_wrapped=f"<evidence>{label}</evidence>",
            bbox=[int(xyxy[0]), int(xyxy[1]), int(xyxy[2]), int(xyxy[3])],
            confidence=conf,
            monitor_id=monitor_id,
            dpi_scale=1.0,
            visual_hash=f"phash_{xyxy[0]}_{xyxy[1]}_{xyxy[2]}_{xyxy[3]}",
        )

    @staticmethod
    def _overlaps(a: List[int], b: List[int]) -> bool:
        """True when box `a`'s center lies inside box `b` (cheap containment)."""
        cx = (a[0] + a[2]) / 2.0
        cy = (a[1] + a[3]) / 2.0
        return b[0] <= cx <= b[2] and b[1] <= cy <= b[3]

    def _label_from_ocr(self, box: List[int], words: List[dict]) -> Optional[str]:
        """Join OCR words whose center falls inside `box` (left→right, top→bottom)."""
        inside = [
            w for w in words
            if box[0] <= (w["x"] + w["w"] / 2.0) <= box[2]
            and box[1] <= (w["y"] + w["h"] / 2.0) <= box[3]
        ]
        if not inside:
            return None
        inside.sort(key=lambda w: (round(w["y"] / 12.0), w["x"]))
        text = " ".join(w["text"] for w in inside).strip()
        return text or None

    def _ocr_words(self, image: Image.Image) -> List[dict]:
        """Run tesseract (sparse-text TSV) over the full frame. Returns a list of
        {text,x,y,w,h,line} dicts. Empty list on any failure (degrade silently —
        boxes still carry class-name labels)."""
        if not self._tesseract:
            return []
        import subprocess
        import tempfile

        try:
            with tempfile.NamedTemporaryFile(suffix=".png", delete=True) as tf:
                image.convert("RGB").save(tf.name)
                proc = subprocess.run(
                    [self._tesseract, tf.name, "stdout", "--psm", "11", "tsv"],
                    capture_output=True,
                    timeout=float(os.environ.get("KRIA_OMNIPARSER_OCR_TIMEOUT", "8")),
                )
            out = proc.stdout.decode("utf-8", "ignore")
        except Exception as exc:
            print(f"[VISION] OCR unavailable (non-fatal): {exc}")
            return []
        words: List[dict] = []
        lines = out.splitlines()
        if not lines:
            return words
        header = lines[0].split("\t")
        try:
            ci = {name: header.index(name) for name in
                  ["left", "top", "width", "height", "conf", "text", "line_num", "block_num", "par_num"]}
        except ValueError:
            return words
        for row in lines[1:]:
            cols = row.split("\t")
            if len(cols) <= ci["text"]:
                continue
            text = cols[ci["text"]].strip()
            if not text:
                continue
            try:
                conf = float(cols[ci["conf"]])
            except ValueError:
                conf = -1.0
            if conf < 30:
                continue
            try:
                words.append({
                    "text": text,
                    "x": int(cols[ci["left"]]),
                    "y": int(cols[ci["top"]]),
                    "w": int(cols[ci["width"]]),
                    "h": int(cols[ci["height"]]),
                    "line": (cols[ci["block_num"]], cols[ci["par_num"]], cols[ci["line_num"]]),
                })
            except ValueError:
                continue
        return words

    @staticmethod
    def _ocr_lines(words: List[dict]) -> List[Tuple[str, List[int]]]:
        """Group OCR words into text lines with a bounding box."""
        groups: dict = {}
        for w in words:
            groups.setdefault(w["line"], []).append(w)
        out: List[Tuple[str, List[int]]] = []
        for ws in groups.values():
            ws.sort(key=lambda w: w["x"])
            text = " ".join(w["text"] for w in ws).strip()
            x1 = min(w["x"] for w in ws)
            y1 = min(w["y"] for w in ws)
            x2 = max(w["x"] + w["w"] for w in ws)
            y2 = max(w["y"] + w["h"] for w in ws)
            out.append((text, [x1, y1, x2, y2]))
        return out

    def _caption_region(self, image: Image.Image, bbox: List[int]) -> Optional[str]:
        if self._caption is None or self._caption_proc is None:
            return None
        try:
            x1, y1, x2, y2 = bbox
            crop = image.crop((x1, y1, max(x1 + 1, x2), max(y1 + 1, y2))).convert("RGB")
            task = "<CAPTION>"
            inputs = self._caption_proc(text=task, images=crop, return_tensors="pt")
            out = self._caption.generate(
                input_ids=inputs["input_ids"],
                pixel_values=inputs["pixel_values"],
                max_new_tokens=48,
                num_beams=3,
                do_sample=False,
            )
            raw = self._caption_proc.batch_decode(out, skip_special_tokens=False)[0]
            # Florence-2 requires task-aware post-processing to extract the text.
            parsed = self._caption_proc.post_process_generation(
                raw, task=task, image_size=(crop.width, crop.height)
            )
            text = ""
            if isinstance(parsed, dict):
                val = parsed.get(task)
                text = (val if isinstance(val, str) else str(val)).strip()
            return text[:200] or None
        except Exception as exc:
            print(f"[VISION] caption region failed (non-fatal): {exc}")
            return None


def get_model():
    """Get or initialize the vision model.

    Selected by `KRIA_VISION_MODEL`: `omniparser` → lightweight YOLO icon
    detector (+ optional caption); `vl7b` → VL-7B grounding; anything else
    (default) → the dummy stub (which reports `degraded=True`).
    """
    global _model
    if _model is None:
        mode = os.environ.get("KRIA_VISION_MODEL", "").strip().lower()
        if mode == "omniparser":
            _model = OmniParser()
        elif mode == "vl7b":
            _model = Vl7bOmniParser()
        else:
            _model = DummyOmniParser()
    return _model


# ============================================================================
# API Endpoints
# ============================================================================

@app.on_event("startup")
async def startup_event():
    """Initialize model on startup."""
    model = get_model()
    print(f"[VISION] OmniParser model loaded: {model.model_name}")


@app.get("/health", response_model=HealthResponse)
async def health_check():
    """Health check endpoint."""
    return HealthResponse(
        status="healthy",
        version="0.1.0",
        model_loaded=_model is not None
    )


@app.post("/parse_screen", response_model=ParseResponse)
async def parse_screen(
    image: UploadFile = File(..., description="Screenshot image (PNG/JPEG)"),
    monitor_id: int = 0,
    confidence_threshold: float = 0.8
):
    """
    Parse a screenshot and return detected UI elements.
    
    This endpoint accepts a raw screenshot image and returns structured
    element data per RFC 007 specification. The output includes bounding
    boxes, confidence scores, and visual hashes for verification.
    """
    start_time = time.time()
    
    try:
        # Read uploaded image
        contents = await image.read()
        img = Image.open(io.BytesIO(contents))
        
        # Ensure RGB format
        if img.mode != 'RGB':
            img = img.convert('RGB')
        
        # Parse with model
        model = get_model()
        output = model.parse(img, monitor_id=monitor_id)
        
        # Filter by confidence threshold
        output.elements = [
            e for e in output.elements 
            if e.confidence >= confidence_threshold
        ]
        
        processing_time = (time.time() - start_time) * 1000
        
        print(f"[VISION] Parsed screenshot: {img.size}, found {len(output.elements)} elements, "
              f"took {processing_time:.1f}ms")
        
        return ParseResponse(
            success=True,
            data=output,
            processing_time_ms=processing_time
        )
        
    except Exception as e:
        print(f"[VISION] Error parsing screenshot: {e}")
        raise HTTPException(status_code=500, detail=f"Parse failed: {str(e)}")


@app.post("/verify_hash")
async def verify_hash(
    image: UploadFile = File(...),
    expected_hash: str = "",
    bbox: Optional[List[int]] = None
):
    """
    Verify visual hash of image region.
    
    Used by click_element tool to verify UI hasn't shifted before clicking.
    Returns similarity score between 0.0 and 1.0.
    """
    try:
        contents = await image.read()
        img = Image.open(io.BytesIO(contents))
        
        model = get_model()
        
        if bbox:
            region_hash = model._calculate_phash(img, bbox)
        else:
            region_hash = model._calculate_fullscreen_hash(img)
        
        # Calculate similarity (mock: exact match = 1.0, else 0.5)
        similarity = 1.0 if region_hash == expected_hash else 0.5
        
        return {
            "similarity": similarity,
            "calculated_hash": region_hash,
            "expected_hash": expected_hash,
            "verified": similarity > 0.90
        }
        
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Hash verification failed: {str(e)}")


@app.get("/")
async def root():
    """Root endpoint with service info."""
    return {
        "service": "KRIA Vision Sidecar",
        "version": "0.1.0",
        "endpoints": [
            "/health",
            "/parse_screen",
            "/parse",
            "/verify_hash"
        ]
    }


# ============================================================================
# GUI Cognition V2 — Sight layer (/parse)
#
# A clean, V2-shaped endpoint that returns the canonical `Observation` the Rust
# `OmniParserSight` consumes directly (ids as ints, bbox as {x,y,width,height},
# kind/label/interactable/confidence), plus an optional Set-of-Mark overlay.
#
# It reuses the existing model backends (DummyOmniParser / Vl7bOmniParser /
# OmniParser) via `get_model()`. A real, lightweight OmniParser backend
# (YOLO icon detector + caption) is added below and selected with
# `KRIA_VISION_MODEL=omniparser`; when its weights/deps are missing it degrades
# HONESTLY (no fabricated elements). The original /parse_screen endpoint and the
# V1 contract are left untouched.
# ============================================================================

from PIL import ImageDraw, ImageFont  # noqa: E402


class V2Bbox(BaseModel):
    x: int
    y: int
    width: int
    height: int


class V2Element(BaseModel):
    id: int
    bbox: V2Bbox
    monitor_index: int = 0
    kind: str
    label: str
    interactable: bool = True
    confidence: float = 0.0


class V2Observation(BaseModel):
    observation_id: str
    screenshot_path: str = ""
    screen_w: int
    screen_h: int
    active_window: Optional[str] = None
    elements: List[V2Element] = Field(default_factory=list)
    som_image_path: Optional[str] = None
    source: str = ""


class ParseV2Request(BaseModel):
    """JSON body for /parse. If `screenshot_b64` is absent the sidecar captures
    the screen itself (via mss) — degrading honestly if capture is unavailable."""
    screenshot_b64: Optional[str] = None
    want_som: bool = False
    monitor_id: int = 0


def _kind_from_element_type(element_type: str) -> str:
    et = (element_type or "").strip().lower()
    mapping = {
        "input": "text_field",
        "textbox": "text_field",
        "text_field": "text_field",
        "button": "button",
        "link": "link",
        "checkbox": "checkbox",
        "tab": "tab",
        "menu": "menu",
        "dialog": "dialog",
        "icon": "icon",
        "text": "text",
    }
    return mapping.get(et, et or "unknown")


def _interactable(kind: str) -> bool:
    return kind in {"button", "text_field", "link", "checkbox", "tab", "menu", "icon"}


def _capture_screen() -> Image.Image:
    """Capture the primary screen. Raises if no capture backend is available so
    the caller can degrade honestly (NEVER fabricate a blank screen)."""
    try:
        import mss  # type: ignore

        with mss.mss() as sct:
            monitor = sct.monitors[1] if len(sct.monitors) > 1 else sct.monitors[0]
            raw = sct.grab(monitor)
            return Image.frombytes("RGB", raw.size, raw.bgra, "raw", "BGRX")
    except Exception as exc:
        raise RuntimeError(f"screen capture unavailable: {exc}")


def _render_set_of_mark(image: Image.Image, elements: List[V2Element]) -> str:
    """Draw numbered boxes for each element and save a PNG; return its path."""
    annotated = image.convert("RGB").copy()
    draw = ImageDraw.Draw(annotated)
    for el in elements:
        b = el.bbox
        x1, y1, x2, y2 = b.x, b.y, b.x + b.width, b.y + b.height
        draw.rectangle([x1, y1, x2, y2], outline=(255, 0, 0), width=2)
        tag = str(el.id)
        # Small filled label box at the top-left corner of the element.
        draw.rectangle([x1, max(0, y1 - 16), x1 + 8 * len(tag) + 6, y1], fill=(255, 0, 0))
        draw.text((x1 + 3, max(0, y1 - 15)), tag, fill=(255, 255, 255))
    out_dir = os.environ.get("KRIA_VISION_SOM_DIR", "/tmp")
    path = os.path.join(out_dir, f"kria_som_{uuid.uuid4().hex}.png")
    annotated.save(path)
    return path


@app.post("/parse", response_model=V2Observation)
async def parse_v2(req: ParseV2Request):
    """GUI Cognition V2 Sight endpoint: screen → canonical Observation."""
    observation_id = uuid.uuid4().hex

    # 1. Obtain the screenshot (provided or captured). Degrade honestly on failure.
    try:
        if req.screenshot_b64:
            raw = base64.b64decode(req.screenshot_b64)
            img = Image.open(io.BytesIO(raw))
        else:
            img = _capture_screen()
        if img.mode != "RGB":
            img = img.convert("RGB")
    except Exception as exc:
        print(f"[VISION] /parse capture degraded: {exc}")
        return V2Observation(
            observation_id=observation_id,
            screen_w=0,
            screen_h=0,
            elements=[],
            source=f"degraded:capture_unavailable:{exc}",
        )

    width, height = img.size

    # 2. Run the selected model backend; convert to V2 elements.
    try:
        model = get_model()
        output = model.parse(img, monitor_id=req.monitor_id)
        if getattr(output, "degraded", False):
            return V2Observation(
                observation_id=observation_id,
                screen_w=width,
                screen_h=height,
                elements=[],
                source=f"degraded:model_unavailable:{getattr(output, 'model', 'unknown')}",
            )
        elements: List[V2Element] = []
        for i, e in enumerate(output.elements, start=1):
            x1, y1, x2, y2 = e.bbox[0], e.bbox[1], e.bbox[2], e.bbox[3]
            kind = _kind_from_element_type(e.element_type)
            elements.append(
                V2Element(
                    id=i,
                    bbox=V2Bbox(x=x1, y=y1, width=max(0, x2 - x1), height=max(0, y2 - y1)),
                    monitor_index=getattr(e, "monitor_id", 0),
                    kind=kind,
                    label=e.label,
                    interactable=_interactable(kind),
                    confidence=e.confidence,
                )
            )
    except Exception as exc:
        print(f"[VISION] /parse model degraded: {exc}")
        return V2Observation(
            observation_id=observation_id,
            screen_w=width,
            screen_h=height,
            elements=[],
            source=f"degraded:model_error:{exc}",
        )

    # 3. Optional Set-of-Mark overlay.
    som_path: Optional[str] = None
    if req.want_som and elements:
        try:
            som_path = _render_set_of_mark(img, elements)
        except Exception as exc:
            print(f"[VISION] /parse SoM render failed (non-fatal): {exc}")

    return V2Observation(
        observation_id=observation_id,
        screen_w=width,
        screen_h=height,
        active_window=None,
        elements=elements,
        som_image_path=som_path,
        source=f"omniparser:{getattr(output, 'model', 'unknown')}",
    )


if __name__ == "__main__":
    import uvicorn
    import os
    port = int(os.environ.get("KRIA_VISION_PORT", "8080"))
    uvicorn.run(app, host="0.0.0.0", port=port)
