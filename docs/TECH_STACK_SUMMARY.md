# K.R.I.A. Tools And Technology Stack Summary

This document summarizes the main tools, libraries, runtimes, and external engines currently used across the K.R.I.A. workspace.

## Rust Core

- **Rust 2021** is the primary systems language for the local assistant runtime, orchestration, tools, safety, memory, and desktop backend.
- **Tokio** is the async runtime used for background tasks, subprocess management, HTTP calls, streaming, cancellation, and health loops.
- **Serde / serde_json / toml** provide serialization for config files, tool schemas, JSON-RPC payloads, events, and API messages.
- **Reqwest** is the main HTTP client for llama-server, ComfyUI, web tools, cloud image fallback, Telegram, and OpenAI-compatible APIs.
- **Axum** powers the secondary HTTP/WebSocket server and some desktop-local API surfaces.
- **Rusqlite** stores local conversations, memory, facts, audit-style records, news data, and indexes.
- **Tracing / tracing-subscriber / tracing-appender** provide structured logging and runtime observability.
- **FastEmbed** provides semantic routing embeddings, currently using `multilingual-e5-small`.
- **ort** integrates ONNX Runtime for local embedding and voice-related ONNX inference paths.
- **ndarray** shapes tensors for ONNX model inputs and outputs.
- **llguidance** is included for grammar-constrained LLM decoding support.
- **NVML / nvidia-smi / sysinfo** provide hardware, RAM, and VRAM telemetry for orchestration.
- **tokio-tungstenite** bridges WebSocket progress from image generation workflows.
- **tokio-util** provides cancellation tokens for LLM streams and turn cancellation.
- **DashMap / once_cell** support shared runtime caches, registries, and lazily initialized global state.
- **cpal / rodio** support local audio capture and playback.
- **whisper-rs** is an optional native speech-to-text backend behind feature flags.
- **webrtc-audio-processing** is an optional acoustic echo cancellation backend behind feature flags.
- **seccompiler** installs Linux seccomp-BPF filtering for the desktop runtime on Linux.
- **fjall / rmp-serde** provide durable embedded queue storage for inbox-style pipelines.

## Python Sidecar

- **Python 3.11+** is the runtime for the `kria-modules` sidecar.
- **JSON-RPC 2.0 over stdin/stdout** is the transport between Rust and the Python sidecar.
- **uv** is preferred for creating and installing the managed sidecar virtual environment.
- **setuptools** builds and packages the Python sidecar module.
- **Pillow** handles image loading, metadata, conversion, and preprocessing.
- **opencv-python-headless** performs image analysis, OCR preprocessing, layout detection, and enhancement.
- **pytesseract** provides standard-tier OCR through the Python sidecar.
- **PyMuPDF** extracts PDF text and document structure.
- **python-docx** extracts DOCX document content.
- **pandas** analyzes tabular data and spreadsheet-like documents.
- **openpyxl** supports XLSX workbook parsing.
- **tree-sitter** provides AST-oriented source code analysis.
- **trafilatura** extracts clean article and webpage text.
- **readability-lxml** provides fallback readability extraction for web pages.
- **sentence-transformers** generates Python-side embeddings for documents, news, and batch processing.
- **librosa** loads and analyzes audio for preprocessing.
- **noisereduce** performs audio cleanup in the sidecar.
- **httpx** powers async HTTP calls in the Telegram MCP sidecar server.
- **easyocr / torch** are optional heavy OCR dependencies for higher hardware tiers.
- **pytest / pytest-asyncio** provide the Python sidecar test environment.

## External Engines And System Tools

- **llama-server** is the managed llama.cpp inference server for local LLM serving, Router Mode load/unload, health checks, and slot save/restore.
- **ComfyUI** is the managed headless image generation engine controlled through REST endpoints.
- **MCP servers** are spawned as stdio JSON-RPC child processes and dynamically bridged into the K.R.I.A. tool registry.
- **whisper.cpp** is the current CLI speech-to-text backend when native `whisper-rs` is not enabled.
- **Piper** is the current CLI text-to-speech backend when in-process TTS is not enabled.
- **Tesseract CLI** is an OCR fallback when sidecar OCR is unavailable.
- **pdftotext** is a native fallback for PDF extraction.
- **Pandoc** is a native fallback for DOCX and document conversion workflows.
- **DuckDuckGo Lite** is used for no-key web search.
- **Open-Meteo** is used for weather lookup.
- **RSS / Atom feeds** are used for basic news retrieval.
- **Pollinations.ai** is the no-key cloud image fallback.
- **HuggingFace Inference API** is an optional token-gated image fallback.
- **OpenAI-compatible HTTP APIs** are supported as configurable external LLM backends.
- **nvidia-smi** is used for NVIDIA GPU detection and telemetry fallback.
- **rocm-smi** is probed for AMD GPU telemetry paths.
- **xdotool / wmctrl / xdg-open / gio** support Linux desktop automation and application control.
- **notify-send / paplay** support Linux notification and sound feedback paths.
- **systemctl / nmcli / wpctl / pactl / amixer / brightnessctl / xrandr** support system configuration tools.
- **apt / dnf / winget / brew / flatpak / snap** are supported by package-management tools where available.

## UI Bindings And Frontend

- **Tauri v2** is the native desktop shell and Rust-to-frontend command bridge.
- **Tauri plugins** provide dialog, notification, clipboard, global shortcut, filesystem, shell, process, and autostart capabilities.
- **SolidJS** is the frontend UI framework.
- **TypeScript** is the UI language layer.
- **Vite** builds and serves the frontend during development.
- **@tauri-apps/api** lets the frontend invoke Rust commands and subscribe to backend events.
- **marked** renders markdown chat content.
- **highlight.js** renders syntax-highlighted code blocks.
- **DOMPurify** sanitizes rendered HTML content.
- **Vitest / jsdom / Solid Testing Library** provide the frontend test environment.

## Storage, Config, And Runtime Layout

- **Cargo workspace** organizes `kria-core`, `kria-desktop`, and `kria-server`.
- **config/default.toml** provides the primary checked-in runtime configuration defaults.
- **models/** stores local model artifacts while large binaries remain gitignored.
- **kria-modules/** contains the Python sidecar package and processors.
- **ui/** contains the SolidJS frontend application.
- **docs/** and **ai-context/** contain human-facing and AI-assistant-facing architecture context.
