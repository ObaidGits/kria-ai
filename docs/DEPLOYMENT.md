# KRIA Deployment

> **Last Updated:** 2026-05-11

---

## Production Build

### Using Build Script

```bash
bash scripts/build-release.sh --features nvidia
```

### Manual Build

```bash
cd ui && npm run build && cd ..
cd crates/kria-desktop && cargo tauri build --features nvidia
```

---

## Output Artifacts

| Platform | Artifacts | Location |
|----------|----------|----------|
| Linux | `.deb`, `.AppImage` | `target/release/bundle/` |
| macOS | `.dmg` | `target/release/bundle/` |
| Windows | `.msi`, `.exe` | `target/release/bundle/` |

---

## Requirements

### Runtime Dependencies

- `llama-server` on PATH (for local LLM)
- Model files in `models/llm/`
- NVIDIA drivers (for GPU mode)

### Optional

- Docker (for OpenClaw skills)
- ComfyUI (for image generation)

---

## Auto-Update

KRIA includes built-in auto-update via Tauri:

```toml
[updater]
enabled = true
endpoint = "https://releases.kria.ai/{{target}}/{{arch}}/{{current_version}}"
```

---

## Installation

### Linux

```bash
# Debian/Ubuntu
sudo dpkg -i kria-desktop_*.deb

# AppImage
chmod +x kria-desktop_*.AppImage
./kria-desktop_*.AppImage
```

### macOS

```bash
# DMG
open kria-desktop_*.dmg
# Drag to Applications
```

### Windows

```powershell
# MSI
msiexec /i kria-desktop_*.msi
```

---

## Configuration

User config stored in:
- Linux/macOS: `~/.kria/config.toml`
- Windows: `%APPDATA%\kria\config.toml`

Database stored in:
- Linux/macOS: `~/.kria/kria.db`
- Windows: `%APPDATA%\kria\kria.db`
