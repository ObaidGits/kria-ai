#!/usr/bin/env python3
"""GUI Cognition TestSubstrate helpers (Task 0.3).

The **TestSubstrate** is the isolated environment where destructive and approval
live tests run without touching the user's real session or data (Requirement 20).
It is composed of three safety primitives, all implemented here as small, pure,
unit-testable pieces so they can be exercised with no display and no network:

1. **Scratch sandbox** — a throw-away directory tree (scratch ``HOME`` with
   ``Downloads`` / ``Documents`` and sample files) that destructive file actions
   are confined to (Requirement 20.2). ``build_scratch`` refuses to operate on or
   near the real ``$HOME``.

2. **Clipboard save/restore** — the user's clipboard is captured before the run
   and restored afterwards (Requirements 7.2, 20.2). The capture/restore is
   best-effort and backend-agnostic (Wayland ``wl-clipboard`` or X11 ``xclip``),
   with the logic split from the subprocess calls so it is testable with a fake
   backend.

3. **Substrate marker env** — the set of environment variables that mark a KRIA
   desktop process as confined to the substrate so the backend gates auto-approval
   fixtures to the substrate only (Requirement 20.3). These MUST mirror the Rust
   constants in
   ``crates/kria-core/src/agent/gui_cognition/execution_environment.rs``.

The shell launcher ``scripts/gui_cognition_test_substrate.sh`` stands up the
display (nested compositor or Xvfb for CI), then calls into this module for the
scratch layout, clipboard handling, and env marker.

CLI
---
    python3 testing/tools/gui_cognition_substrate.py --setup-scratch DIR
    python3 testing/tools/gui_cognition_substrate.py --print-env --scratch-dir DIR
    python3 testing/tools/gui_cognition_substrate.py --clipboard-save  > /tmp/clip.b64
    python3 testing/tools/gui_cognition_substrate.py --clipboard-restore < /tmp/clip.b64
"""
from __future__ import annotations

import argparse
import base64
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

# These MUST stay in sync with the Rust constants in
# crates/kria-core/src/agent/gui_cognition/execution_environment.rs
SUBSTRATE_ENV_FLAG = "KRIA_GUI_TEST_SUBSTRATE"
SUBSTRATE_ENV_SCRATCH_DIR = "KRIA_GUI_TEST_SUBSTRATE_SCRATCH_DIR"
SUBSTRATE_ENV_RESTORE_CLIPBOARD = "KRIA_GUI_TEST_SUBSTRATE_RESTORE_CLIPBOARD"


# ---------------------------------------------------------------------------
# Scratch sandbox (Requirement 20.2)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SampleFile:
    """A sample file created in the scratch sandbox."""

    relative_path: str
    content: str


# Sample files give destructive/select prompts something safe to act on
# (e.g. "select the newest file in Downloads"). They are deliberately boring.
DEFAULT_SAMPLE_FILES: tuple[SampleFile, ...] = (
    SampleFile("Downloads/sample-report.txt", "scratch downloads sample report\n"),
    SampleFile("Downloads/notes.md", "# Scratch Notes\n\nThrow-away file.\n"),
    SampleFile("Downloads/archive-old.txt", "older scratch file for newest-file tests\n"),
    SampleFile("Documents/todo.txt", "- scratch todo item\n"),
    SampleFile("Documents/draft.md", "# Draft\n\nScratch document.\n"),
)

DEFAULT_SCRATCH_DIRS: tuple[str, ...] = (
    "Downloads",
    "Documents",
    "Desktop",
    ".cache",
    ".local/share",
)


@dataclass
class ScratchManifest:
    """Result of building the scratch sandbox."""

    root: Path
    home: Path
    dirs: list[Path] = field(default_factory=list)
    files: list[Path] = field(default_factory=list)

    def summary(self) -> str:
        return (
            f"scratch root={self.root} home={self.home} "
            f"dirs={len(self.dirs)} files={len(self.files)}"
        )


class UnsafeScratchError(RuntimeError):
    """Raised when a scratch path would collide with the user's real data."""


def _real_home() -> Path:
    return Path(os.path.expanduser("~")).resolve()


def assert_scratch_path_is_safe(root: Path) -> None:
    """Refuse scratch roots that would endanger the user's real data.

    The scratch root must NOT be the real ``$HOME`` nor an ancestor of it, and
    the real ``$HOME`` must not live inside the scratch root. This guarantees a
    destructive action confined to the scratch tree can never reach real files.
    """
    root = root.resolve()
    home = _real_home()
    if root == home:
        raise UnsafeScratchError(f"scratch root {root} is the real HOME; refusing")
    if root in home.parents:
        raise UnsafeScratchError(
            f"scratch root {root} is an ancestor of the real HOME {home}; refusing"
        )
    if home == root or home in root.parents:
        # Real HOME contains the scratch root only if scratch is *inside* HOME,
        # which is allowed ONLY under a clearly-scratch subdir name.
        if "substrate" not in root.name and "scratch" not in str(root).lower():
            raise UnsafeScratchError(
                f"scratch root {root} is inside the real HOME but is not clearly a "
                "scratch directory; refusing"
            )
    for danger in ("/", str(home)):
        if str(root) == danger:
            raise UnsafeScratchError(f"scratch root {root} is a protected path; refusing")


def build_scratch(
    root: os.PathLike[str] | str,
    *,
    sample_files: tuple[SampleFile, ...] = DEFAULT_SAMPLE_FILES,
    dirs: tuple[str, ...] = DEFAULT_SCRATCH_DIRS,
    clean: bool = True,
) -> ScratchManifest:
    """Create the scratch sandbox tree and sample files.

    The layout is a throw-away ``HOME`` (the root itself) with the standard XDG
    user dirs and a handful of sample files. Returns a manifest of what was made.
    """
    root_path = Path(root).resolve()
    assert_scratch_path_is_safe(root_path)

    if clean and root_path.exists():
        shutil.rmtree(root_path)
    root_path.mkdir(parents=True, exist_ok=True)

    manifest = ScratchManifest(root=root_path, home=root_path)
    for rel in dirs:
        d = root_path / rel
        d.mkdir(parents=True, exist_ok=True)
        manifest.dirs.append(d)

    for spec in sample_files:
        f = root_path / spec.relative_path
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(spec.content, encoding="utf-8")
        manifest.files.append(f)

    return manifest


def teardown_scratch(root: os.PathLike[str] | str) -> bool:
    """Remove the scratch sandbox tree. Returns True if anything was removed."""
    root_path = Path(root).resolve()
    assert_scratch_path_is_safe(root_path)
    if root_path.exists():
        shutil.rmtree(root_path)
        return True
    return False


# ---------------------------------------------------------------------------
# Substrate marker env (Requirement 20.3)
# ---------------------------------------------------------------------------


def substrate_env(scratch_dir: os.PathLike[str] | str, *, restore_clipboard: bool = True) -> dict[str, str]:
    """Return the env vars that mark a process as a confined test substrate.

    These are consumed server-side by KRIA's
    ``GuiExecutionEnvironment::from_env`` to gate auto-approval fixtures.
    """
    return {
        SUBSTRATE_ENV_FLAG: "1",
        SUBSTRATE_ENV_SCRATCH_DIR: str(Path(scratch_dir).resolve()),
        SUBSTRATE_ENV_RESTORE_CLIPBOARD: "1" if restore_clipboard else "0",
    }


def is_substrate_env(env: dict[str, str]) -> bool:
    """Whether an env mapping marks a confined substrate (mirrors the Rust gate)."""
    return env.get(SUBSTRATE_ENV_FLAG, "").strip().lower() in {"1", "true", "yes", "on"}


# ---------------------------------------------------------------------------
# Clipboard save/restore (Requirements 7.2, 20.2)
# ---------------------------------------------------------------------------


@dataclass
class ClipboardTools:
    """Resolved clipboard backend command tuples (or ``None`` when absent)."""

    backend: str  # "wayland" | "x11" | "none"
    read_cmd: list[str] | None
    write_cmd: list[str] | None


def detect_clipboard_tools(
    env: dict[str, str] | None = None,
    which: Callable[[str], str | None] = shutil.which,
) -> ClipboardTools:
    """Pick a clipboard backend from the environment + available binaries.

    Prefers Wayland (``wl-paste``/``wl-copy``) when ``WAYLAND_DISPLAY`` is set,
    otherwise X11 (``xclip``) when ``DISPLAY`` is set. Pure given ``which``.
    """
    env = env if env is not None else dict(os.environ)
    if env.get("WAYLAND_DISPLAY") and which("wl-paste") and which("wl-copy"):
        return ClipboardTools(
            backend="wayland",
            read_cmd=["wl-paste", "--no-newline"],
            write_cmd=["wl-copy"],
        )
    if env.get("DISPLAY") and which("xclip"):
        return ClipboardTools(
            backend="x11",
            read_cmd=["xclip", "-selection", "clipboard", "-o"],
            write_cmd=["xclip", "-selection", "clipboard", "-i"],
        )
    return ClipboardTools(backend="none", read_cmd=None, write_cmd=None)


def save_clipboard(
    tools: ClipboardTools | None = None,
    runner: Callable[..., subprocess.CompletedProcess] = subprocess.run,
) -> bytes | None:
    """Capture the current clipboard contents (best-effort).

    Returns the raw bytes, or ``None`` when no backend is available or capture
    fails. Never raises — a missing clipboard must not abort the substrate.
    """
    tools = tools or detect_clipboard_tools()
    if not tools.read_cmd:
        return None
    try:
        result = runner(tools.read_cmd, capture_output=True, check=False, timeout=10)
        if result.returncode != 0:
            return None
        return result.stdout if isinstance(result.stdout, bytes) else str(result.stdout).encode()
    except Exception:  # noqa: BLE001 — best-effort; never abort
        return None


def restore_clipboard(
    data: bytes | None,
    tools: ClipboardTools | None = None,
    runner: Callable[..., subprocess.CompletedProcess] = subprocess.run,
) -> bool:
    """Restore previously-captured clipboard contents (best-effort).

    Returns True on success. ``None`` data is a no-op (nothing to restore).
    Never raises.
    """
    if data is None:
        return False
    tools = tools or detect_clipboard_tools()
    if not tools.write_cmd:
        return False
    try:
        result = runner(tools.write_cmd, input=data, check=False, timeout=10)
        return getattr(result, "returncode", 1) == 0
    except Exception:  # noqa: BLE001 — best-effort; never abort
        return False


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="GUI Cognition TestSubstrate helper")
    ap.add_argument("--setup-scratch", metavar="DIR", help="create the scratch sandbox at DIR")
    ap.add_argument("--teardown-scratch", metavar="DIR", help="remove the scratch sandbox at DIR")
    ap.add_argument("--print-env", action="store_true", help="print substrate marker env (sh export lines)")
    ap.add_argument("--scratch-dir", metavar="DIR", help="scratch dir for --print-env")
    ap.add_argument("--no-restore-clipboard", action="store_true")
    ap.add_argument("--clipboard-save", action="store_true", help="capture clipboard, emit base64 to stdout")
    ap.add_argument("--clipboard-restore", action="store_true", help="restore clipboard from base64 on stdin")
    args = ap.parse_args(argv)

    if args.setup_scratch:
        try:
            manifest = build_scratch(args.setup_scratch)
        except UnsafeScratchError as exc:
            print(f"refusing unsafe scratch dir: {exc}", file=sys.stderr)
            return 2
        print(manifest.summary())
        return 0

    if args.teardown_scratch:
        try:
            removed = teardown_scratch(args.teardown_scratch)
        except UnsafeScratchError as exc:
            print(f"refusing unsafe scratch dir: {exc}", file=sys.stderr)
            return 2
        print("removed" if removed else "nothing to remove")
        return 0

    if args.print_env:
        if not args.scratch_dir:
            print("--print-env requires --scratch-dir", file=sys.stderr)
            return 2
        env = substrate_env(args.scratch_dir, restore_clipboard=not args.no_restore_clipboard)
        for key, value in env.items():
            print(f"export {key}={value}")
        return 0

    if args.clipboard_save:
        data = save_clipboard()
        sys.stdout.write(base64.b64encode(data or b"").decode("ascii"))
        return 0

    if args.clipboard_restore:
        raw = sys.stdin.read().strip()
        data = base64.b64decode(raw) if raw else None
        ok = restore_clipboard(data)
        return 0 if ok else 1

    ap.print_help()
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
