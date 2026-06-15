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
        self.model_name = "qwen2.5-vl-7b"
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


def get_model():
    """Get or initialize the vision model.

    Selected by `KRIA_VISION_MODEL` (Task 8): `vl7b` → real VL-7B grounding;
    anything else (default) → the dummy stub (which reports `degraded=True`).
    """
    global _model
    if _model is None:
        mode = os.environ.get("KRIA_VISION_MODEL", "").strip().lower()
        if mode == "vl7b":
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
            "/verify_hash"
        ]
    }


if __name__ == "__main__":
    import uvicorn
    import os
    port = int(os.environ.get("KRIA_VISION_PORT", "8080"))
    uvicorn.run(app, host="0.0.0.0", port=port)
