# K.R.I.A. Product Overview

## What is K.R.I.A.?

**K.R.I.A.** (Kernel Responsive Intelligent Agent) is a local-first AI desktop assistant that combines conversational AI, workflow automation, and context-aware execution into a unified platform.

### Core Value Proposition

- **Local-First & Privacy-Focused**: Runs entirely on your machine; your data never leaves your device
- **Voice-Native**: Hands-free interaction with voice control, STT, and TTS
- **Extensible**: Install skills from ClawHub marketplace; sandboxed execution with capability control
- **Intelligent**: Multi-step reasoning with tool orchestration, intent classification, and adaptive decision-making
- **Safe**: Risk classification (GREEN/YELLOW/RED/BLACK), human-in-the-loop approval, audit logging, and rollback support

### Key Capabilities

- **60+ Native Tools**: System info, file operations, web search, document parsing, package management, process control, shell execution, knowledge/RAG, communication, Google Workspace integration, image generation, and more
- **OpenClaw Skill Substrate**: Sandboxed Docker-based skills with trust tiers and capability control
- **Advanced Voice Pipeline**: Whisper STT, Piper TTS, WebRTC VAD, wake word detection, acoustic echo cancellation, and real-time streaming
- **Image Generation**: Local GPU-accelerated generation via ComfyUI with cloud fallback (DALL-E, Stability AI)
- **Fleet & Remote Execution**: SSH-based remote command execution on enrolled targets with QoS scheduling and snapshot orchestration
- **Memory & Knowledge**: Persistent conversation history, facts with decay scoring, RAG-indexed documents, and semantic search

### Target Users

- Power users seeking privacy-first automation
- Developers building AI-driven workflows
- Organizations needing local AI infrastructure
- Users wanting extensible, skill-based AI assistants

### Platform Support

- Linux, macOS, Windows (desktop)
- Server deployment via Axum HTTP/WebSocket APIs
- Remote execution via SSH and QEMU
