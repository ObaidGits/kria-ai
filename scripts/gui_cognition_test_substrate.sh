#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────
# K.R.I.A. — GUI Cognition TestSubstrate launcher (spec task 0.3)
# ─────────────────────────────────────────────────────────────────
# Stands up an ISOLATED substrate where destructive / approval GUI
# Cognition live tests run WITHOUT touching the user's real session or
# data (Requirement 20):
#
#   • a separate display  — a nested Wayland compositor (weston/labwc/
#     sway) on a real desktop, or Xvfb for headless / CI seats;
#   • a scratch HOME       — throw-away Downloads/Documents + sample files
#                            (destructive file actions are confined here);
#   • clipboard save/restore — the user's clipboard is captured before the
#                            run and restored at teardown (best effort);
#   • the substrate marker — KRIA_GUI_TEST_SUBSTRATE=1 so the backend gates
#                            auto-approval fixtures to the substrate only
#                            (Requirement 20.3). Auto-approval can NEVER
#                            execute on the real session.
#
# The scratch layout, clipboard handling, and env marker are implemented
# in testing/tools/gui_cognition_substrate.py (unit-tested, no display).
#
# Usage:
#   scripts/gui_cognition_test_substrate.sh [options] [-- command ...]
#
# Options:
#   --mode {auto|xvfb|nested}  display backend (default: auto)
#   --display :N               display number (default: :99)
#   --scratch-dir DIR          scratch HOME (default: $TMPDIR/kria-gui-substrate)
#   --resolution WxHxD         Xvfb screen geometry (default: 1920x1200x24)
#   --no-restore-clipboard     skip clipboard save/restore
#   --keep                     do not tear down the scratch dir on exit
#   -- command ...             run command inside the substrate (else prints
#                              an `export` block to source, then exits)
#
# Examples:
#   # Run the live capability audit in the substrate (CI / headless):
#   scripts/gui_cognition_test_substrate.sh --mode xvfb -- \
#       python3 testing/tools/gui_cognition_capability_audit.py \
#       --environment test_substrate --runs 3
#
#   # Print env to source into the current shell (debugging):
#   eval "$(scripts/gui_cognition_test_substrate.sh --mode nested)"
# ─────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SUBSTRATE_PY="$REPO_ROOT/testing/tools/gui_cognition_substrate.py"
PY="${PYTHON:-python3}"

# ── Defaults ──────────────────────────────────────────────────────
MODE="auto"
DISPLAY_NUM=":99"
SCRATCH_DIR="${TMPDIR:-/tmp}/kria-gui-substrate"
RESOLUTION="1920x1200x24"
RESTORE_CLIPBOARD=1
KEEP=0
CMD=()

# ── Parse args ────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode) MODE="$2"; shift 2 ;;
        --display) DISPLAY_NUM="$2"; shift 2 ;;
        --scratch-dir) SCRATCH_DIR="$2"; shift 2 ;;
        --resolution) RESOLUTION="$2"; shift 2 ;;
        --no-restore-clipboard) RESTORE_CLIPBOARD=0; shift ;;
        --keep) KEEP=1; shift ;;
        --) shift; CMD=("$@"); break ;;
        -h|--help) sed -n '2,52p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

log() { echo "[substrate] $*" >&2; }

# ── Safety: never reuse the real desktop display ──────────────────
# The substrate MUST be isolated. Refuse a display number that looks
# like a real session (:0 / :1).
case "$DISPLAY_NUM" in
    :0|:1|:0.*|:1.*)
        echo "REFUSING: $DISPLAY_NUM looks like the real desktop session." >&2
        echo "Pick an isolated display (e.g. :99) for the substrate." >&2
        exit 1
        ;;
esac

# ── Resolve display backend ───────────────────────────────────────
resolve_mode() {
    if [[ "$MODE" != "auto" ]]; then echo "$MODE"; return; fi
    # Prefer a nested compositor on a real session; fall back to Xvfb.
    if [[ -n "${WAYLAND_DISPLAY:-}" || -n "${DISPLAY:-}" ]] && command -v weston >/dev/null 2>&1; then
        echo "nested"; return
    fi
    if command -v Xvfb >/dev/null 2>&1; then echo "xvfb"; return; fi
    echo "none"
}
RESOLVED_MODE="$(resolve_mode)"
log "display backend: $RESOLVED_MODE (requested: $MODE)"

# ── Build the scratch sandbox ─────────────────────────────────────
log "building scratch sandbox at $SCRATCH_DIR"
"$PY" "$SUBSTRATE_PY" --setup-scratch "$SCRATCH_DIR" >&2

# ── Save the user's clipboard (best effort) ───────────────────────
CLIP_TMP=""
if [[ "$RESTORE_CLIPBOARD" -eq 1 ]]; then
    CLIP_TMP="$(mktemp)"
    if "$PY" "$SUBSTRATE_PY" --clipboard-save > "$CLIP_TMP" 2>/dev/null; then
        log "saved user clipboard ($(wc -c < "$CLIP_TMP") b64 bytes)"
    else
        log "clipboard save unavailable (best effort); continuing"
        : > "$CLIP_TMP"
    fi
fi

# ── Display lifecycle ─────────────────────────────────────────────
DISPLAY_PID=""
start_display() {
    case "$RESOLVED_MODE" in
        xvfb)
            command -v Xvfb >/dev/null 2>&1 || { echo "Xvfb not installed (apt install xvfb)" >&2; exit 1; }
            log "starting Xvfb on $DISPLAY_NUM ($RESOLUTION)"
            Xvfb "$DISPLAY_NUM" -screen 0 "$RESOLUTION" -nolisten tcp >/dev/null 2>&1 &
            DISPLAY_PID=$!
            export DISPLAY="$DISPLAY_NUM"
            unset WAYLAND_DISPLAY || true
            sleep 1
            ;;
        nested)
            command -v weston >/dev/null 2>&1 || { echo "weston not installed (apt install weston)" >&2; exit 1; }
            local sock="kria-substrate-${DISPLAY_NUM#:}"
            log "starting nested weston compositor (socket: $sock)"
            weston --socket="$sock" --width=1920 --height=1200 --idle-time=0 >/dev/null 2>&1 &
            DISPLAY_PID=$!
            export WAYLAND_DISPLAY="$sock"
            sleep 1
            ;;
        none)
            echo "No display backend available (need Xvfb or weston)." >&2
            echo "Install one, or use the deterministic fixture tier (task 0.4) for no-display CI." >&2
            exit 1
            ;;
        *)
            echo "unknown mode: $RESOLVED_MODE" >&2; exit 2 ;;
    esac
}

cleanup() {
    local code=$?
    # Restore the user's clipboard first — most user-visible.
    if [[ "$RESTORE_CLIPBOARD" -eq 1 && -n "$CLIP_TMP" && -s "$CLIP_TMP" ]]; then
        if "$PY" "$SUBSTRATE_PY" --clipboard-restore < "$CLIP_TMP" 2>/dev/null; then
            log "restored user clipboard"
        else
            log "clipboard restore unavailable (best effort)"
        fi
    fi
    [[ -n "$CLIP_TMP" ]] && rm -f "$CLIP_TMP" || true
    if [[ -n "$DISPLAY_PID" ]]; then
        log "stopping display backend (pid $DISPLAY_PID)"
        kill "$DISPLAY_PID" >/dev/null 2>&1 || true
        wait "$DISPLAY_PID" 2>/dev/null || true
    fi
    if [[ "$KEEP" -eq 0 ]]; then
        log "tearing down scratch sandbox"
        "$PY" "$SUBSTRATE_PY" --teardown-scratch "$SCRATCH_DIR" >&2 || true
    else
        log "keeping scratch sandbox at $SCRATCH_DIR (--keep)"
    fi
    exit "$code"
}
trap cleanup EXIT INT TERM

# ── Substrate environment ─────────────────────────────────────────
# Confine HOME + XDG dirs to the scratch tree; mark the substrate.
export HOME="$SCRATCH_DIR"
export XDG_DATA_HOME="$SCRATCH_DIR/.local/share"
export XDG_CACHE_HOME="$SCRATCH_DIR/.cache"
export XDG_DOWNLOAD_DIR="$SCRATCH_DIR/Downloads"
export XDG_DOCUMENTS_DIR="$SCRATCH_DIR/Documents"
export KRIA_GUI_TEST_SUBSTRATE=1
export KRIA_GUI_TEST_SUBSTRATE_SCRATCH_DIR="$SCRATCH_DIR"
export KRIA_GUI_TEST_SUBSTRATE_RESTORE_CLIPBOARD="$RESTORE_CLIPBOARD"

if [[ "${#CMD[@]}" -eq 0 ]]; then
    # No command: print an export block the caller can `eval`/source. We do NOT
    # start a display in this mode (the caller manages their own lifecycle).
    trap - EXIT INT TERM
    [[ -n "$CLIP_TMP" ]] && rm -f "$CLIP_TMP" || true
    "$PY" "$SUBSTRATE_PY" --print-env --scratch-dir "$SCRATCH_DIR" \
        $([[ "$RESTORE_CLIPBOARD" -eq 0 ]] && echo --no-restore-clipboard)
    echo "export HOME=$SCRATCH_DIR"
    log "no command given; printed export block (display NOT started)"
    exit 0
fi

start_display
log "running inside substrate: ${CMD[*]}"
"${CMD[@]}"
