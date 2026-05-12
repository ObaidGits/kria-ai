# RFC 007 Phase 4 - E2E GUI Test Instructions

This document describes how to run the safe sandboxed E2E test for the GUI automation system.

## Prerequisites

Before running the test, you need three components running:

### 1. Python Vision Sidecar

The OmniParser screen understanding service:

```bash
cd sidecars/kria-vision

# Create virtual environment (recommended)
python -m venv venv
source venv/bin/activate  # Linux/Mac
# or: venv\Scripts\activate  # Windows

# Install dependencies
pip install -r requirements.txt

# Start the service
python main.py
```

The service will start on `http://localhost:8080`. Verify it's running:
```bash
curl http://localhost:8080/health
```

Expected response:
```json
{"status": "healthy", "version": "0.1.0", "model_loaded": true}
```

### 2. Uinput Daemon (Privileged)

The isolated helper for GUI input injection:

```bash
# Build first
cargo build -p kria-uinput-daemon --release

# Run with sudo (requires uinput kernel access)
sudo ./target/release/kria-uinput-daemon
```

The daemon creates a Unix socket at `/tmp/kria-uinput.sock` with `chmod 600` permissions.

### 3. (Optional) Set Environment Variables

```bash
export KRIA_OMNIPARSER_ENDPOINT="http://localhost:8080"
export KRIA_UINPUT_SOCKET="/tmp/kria-uinput.sock"
```

## Running the E2E Test

Once all prerequisites are running:

```bash
cargo run -p kria-core --bin test_gui_e2e
```

### What the Test Does

1. **Checks prerequisites** - Verifies vision sidecar and uinput daemon are running
2. **Generates HTN workflow** - Creates a GUI automation plan:
   - Open gedit text editor
   - Get screen elements
   - Click text area (with pHash verification)
   - Type: "KRIA HTN E2E TEST SUCCESS"
   - Verify text appeared
3. **Executes workflow** - Runs through GuiExecutor with:
   - Kill-switch protection
   - Bounded micro-retries
   - Safe abort on failure
4. **Reports results** - Shows success/failure with timing

### Expected Output

```
╔══════════════════════════════════════════════════════════════╗
║  KRIA RFC 007 Phase 4 - E2E GUI Automation Test              ║
║  Safe Sandboxed Test: Text Editor Workflow                   ║
╚══════════════════════════════════════════════════════════════╝

🔍 Checking prerequisites...
  ✓ Vision sidecar online at http://localhost:8080
  ✓ Uinput daemon socket found at "/tmp/kria-uinput.sock"

⚠️  WARNING: This test will:
   1. Open gedit text editor on your desktop
   2. Click in the text area
   3. Type: 'KRIA HTN E2E TEST SUCCESS'

Press Ctrl+C within 5 seconds to cancel...

🔧 Initializing tool registry...
  ✓ Registered X tools

📋 Building HTN workflow...
  ✓ Workflow generated: e2e-test-001
     - 4 sub-goals
     - 2 safe abort steps

📋 Workflow details:
  Step 1: open_application (verify: WindowState)
  Step 2: get_screen_elements (verify: ElementsFound)
  Step 3: click_element (verify: ScreenChanged)
  Step 4: type_text (verify: TextPresent)

🔧 Initializing GUI executor...
  ✓ Executor initialized

🚀 Executing HTN workflow...
═══════════════════════════════════════════════════════════════
[Execution logs...]
═══════════════════════════════════════════════════════════════

📊 Test Results:
  Task ID: e2e-test-001
  Success: ✅ PASSED
  Steps: 4/4
  Duration: 5234ms
  Aborted: false

🧹 Cleanup:
  You can close the gedit window if it remains open.

🎉 E2E TEST COMPLETED SUCCESSFULLY!
```

## Troubleshooting

### Vision Sidecar Connection Failed

**Error:** `Vision sidecar NOT reachable`

**Solution:**
```bash
# Check if Python service is running
curl http://localhost:8080/health

# If not running, start it:
cd sidecars/kria-vision
python main.py
```

### Uinput Daemon Socket Not Found

**Error:** `Uinput daemon socket NOT found`

**Solution:**
```bash
# Run with sudo (required for uinput access)
sudo cargo run -p kria-uinput-daemon
```

### Permission Denied on Socket

**Error:** `Failed to connect to daemon: Permission denied`

**Solution:**
The socket should have `chmod 600` permissions. Check:
```bash
ls -la /tmp/kria-uinput.sock
# Should show: srw------- 1 root root ...
```

Run the E2E test with appropriate permissions or adjust socket permissions.

### Workflow Fails at Visual Hash Verification

**Error:** `Visual hash mismatch`

**Solution:**
The UI element may have changed between discovery and click. This is expected behavior - the pHash verification (>0.90 threshold) prevents clicking on moved elements. Check:
1. No animations or popups are interfering
2. Screen is stable during test
3. Try running test on a less dynamic part of the screen

### Text Not Verified

**Error:** `Text verification failed`

**Solution:**
The OCR may not detect the typed text. This can happen if:
1. Font rendering is too small for the vision model
2. Text color blends with background
3. Vision sidecar model needs tuning

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    test_gui_e2e                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 1. Generate HTN Workflow (TurnGate)                  │  │
│  │    {"task_id": "e2e-001", "sub_goals": [...]}         │  │
│  └──────────────────────────────────────────────────────┘  │
│                           │                                 │
│                           ▼                                 │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 2. Execute via GuiExecutor                             │  │
│  │    - KillSwitchInterceptor checks                    │  │
│  │    - Bounded micro-retries                           │  │
│  │    - Safe abort on failure                           │  │
│  └──────────────────────────────────────────────────────┘  │
│                           │                                 │
│           ┌───────────────┼───────────────┐               │
│           ▼               ▼               ▼               │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐      │
│  │ GUI Tools    │ │ Vision Tools │ │ Abort Steps  │      │
│  │ (ydotool)    │ │ (OmniParser) │ │ (Escape)     │      │
│  └──────────────┘ └──────────────┘ └──────────────┘      │
│          │                │                               │
│          ▼                ▼                               │
│  ┌──────────────┐ ┌──────────────┐                       │
│  │ kria-uinput  │ │ kria-vision  │                       │
│  │ -daemon      │ │ (Python)     │                       │
│  │ (Unix Socket)│ │ (HTTP 8080)  │                       │
│  └──────────────┘ └──────────────┘                       │
│          │                │                               │
│          ▼                ▼                               │
│       ┌──────┐         ┌──────┐                           │
│       │uinput│         │Screen│                           │
│       │kernel│         │OCR   │                           │
│       └──────┘         └──────┘                           │
└─────────────────────────────────────────────────────────────┘
```

## Next Steps

After successful E2E test:

1. **Replace dummy model** - Swap `DummyOmniParser` in `sidecars/kria-vision/main.py` with real PyTorch/ONNX model
2. **Tune pHash threshold** - Adjust similarity threshold based on real-world stability
3. **Expand test scenarios** - Add more workflows (form filling, multi-window, etc.)
4. **Integrate with TurnGate** - Connect LLM routing to generate HTN plans dynamically
5. **Add HITL gateway** - Wire up approval system for RED-tier GUI actions

## RFC 007 Compliance Checklist

- ✅ Privilege isolation (uinput daemon)
- ✅ Kill switch interceptor
- ✅ Rate limiting (max 2 actions/sec)
- ✅ Modifier key release on abort
- ✅ Protected mode detection
- ✅ Clipboard atomic backup
- ✅ 5-second vision cache
- ✅ Cache invalidation on state change
- ✅ Visual hash verification (>0.90)
- ✅ Bounded micro-retries (250/500/1000ms)
- ✅ Safe abort sequences
- ✅ HTN workflow immutability
- ✅ Max duration enforcement (5 min)
