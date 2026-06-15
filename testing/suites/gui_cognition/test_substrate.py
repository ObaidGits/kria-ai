"""Unit tests for the GUI Cognition TestSubstrate helpers (Task 0.3).

These cover the substrate's safety primitives WITHOUT a display or network:
* scratch sandbox is built under a throw-away root and confined there,
* the sandbox refuses to operate on / near the user's real HOME,
* clipboard save/restore round-trips through an injected fake backend,
* the substrate marker env mirrors the Rust gate and is truthy only in-substrate,
* auto-approval is permitted ONLY inside the substrate (Requirement 20.3),
* the launcher script exists and is executable.

Run from repo root:
    python3 -m pytest testing/suites/gui_cognition/test_substrate.py
"""
from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from testing.tools.gui_cognition_substrate import (
    DEFAULT_SAMPLE_FILES,
    SUBSTRATE_ENV_FLAG,
    SUBSTRATE_ENV_RESTORE_CLIPBOARD,
    SUBSTRATE_ENV_SCRATCH_DIR,
    ClipboardTools,
    UnsafeScratchError,
    assert_scratch_path_is_safe,
    build_scratch,
    detect_clipboard_tools,
    is_substrate_env,
    restore_clipboard,
    save_clipboard,
    substrate_env,
    teardown_scratch,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
LAUNCHER = REPO_ROOT / "scripts" / "gui_cognition_test_substrate.sh"


# ---------------------------------------------------------------------------
# Scratch sandbox isolation (Requirement 20.2)
# ---------------------------------------------------------------------------


class ScratchSandboxTests(unittest.TestCase):
    def test_build_scratch_creates_dirs_and_sample_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "kria-substrate-scratch"
            manifest = build_scratch(root)
            self.assertTrue((root / "Downloads").is_dir())
            self.assertTrue((root / "Documents").is_dir())
            # Every sample file exists with content.
            for spec in DEFAULT_SAMPLE_FILES:
                f = root / spec.relative_path
                self.assertTrue(f.is_file(), f"missing {f}")
                self.assertEqual(f.read_text(encoding="utf-8"), spec.content)
            self.assertEqual(len(manifest.files), len(DEFAULT_SAMPLE_FILES))

    def test_all_scratch_files_confined_under_root(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "kria-substrate-scratch"
            manifest = build_scratch(root)
            for p in manifest.files + manifest.dirs:
                self.assertTrue(
                    str(p.resolve()).startswith(str(root.resolve())),
                    f"{p} escaped scratch root {root}",
                )

    def test_build_scratch_is_idempotent_clean(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "scratch"
            build_scratch(root)
            # Drop a stray file; a clean rebuild must remove it.
            stray = root / "Downloads" / "stray.bin"
            stray.write_bytes(b"x")
            build_scratch(root, clean=True)
            self.assertFalse(stray.exists())

    def test_teardown_removes_tree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "scratch"
            build_scratch(root)
            self.assertTrue(teardown_scratch(root))
            self.assertFalse(root.exists())
            # Second teardown is a no-op.
            self.assertFalse(teardown_scratch(root))


class ScratchSafetyTests(unittest.TestCase):
    def test_refuses_real_home(self) -> None:
        home = Path(os.path.expanduser("~")).resolve()
        with self.assertRaises(UnsafeScratchError):
            assert_scratch_path_is_safe(home)

    def test_refuses_root(self) -> None:
        with self.assertRaises(UnsafeScratchError):
            assert_scratch_path_is_safe(Path("/"))

    def test_refuses_ancestor_of_home(self) -> None:
        home = Path(os.path.expanduser("~")).resolve()
        # An ancestor of HOME (e.g. /home) must be refused.
        if home.parent != home:
            with self.assertRaises(UnsafeScratchError):
                assert_scratch_path_is_safe(home.parent)

    def test_allows_clearly_scratch_tmp_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            # Should not raise.
            assert_scratch_path_is_safe(Path(tmp) / "kria-gui-substrate")


# ---------------------------------------------------------------------------
# Substrate marker env + auto-approval gate (Requirement 20.3)
# ---------------------------------------------------------------------------


class SubstrateEnvTests(unittest.TestCase):
    def test_substrate_env_sets_flag_and_scratch(self) -> None:
        env = substrate_env("/tmp/kria-gui-substrate", restore_clipboard=True)
        self.assertEqual(env[SUBSTRATE_ENV_FLAG], "1")
        self.assertTrue(env[SUBSTRATE_ENV_SCRATCH_DIR].endswith("kria-gui-substrate"))
        self.assertEqual(env[SUBSTRATE_ENV_RESTORE_CLIPBOARD], "1")

    def test_restore_clipboard_can_be_disabled(self) -> None:
        env = substrate_env("/tmp/x-scratch", restore_clipboard=False)
        self.assertEqual(env[SUBSTRATE_ENV_RESTORE_CLIPBOARD], "0")

    def test_is_substrate_env_truthy_only_in_substrate(self) -> None:
        self.assertTrue(is_substrate_env({SUBSTRATE_ENV_FLAG: "1"}))
        self.assertTrue(is_substrate_env({SUBSTRATE_ENV_FLAG: "true"}))
        # Real session: flag absent or falsey -> auto-approval NOT allowed.
        self.assertFalse(is_substrate_env({}))
        self.assertFalse(is_substrate_env({SUBSTRATE_ENV_FLAG: "0"}))
        self.assertFalse(is_substrate_env({SUBSTRATE_ENV_FLAG: "false"}))

    def test_auto_approval_permitted_only_in_substrate(self) -> None:
        """Mirror of the backend gate: auto-approve allowed iff substrate."""
        real_session = {}
        substrate = substrate_env("/tmp/x-scratch")
        # The decision used by the harness to decide whether to send a fixture.
        allow_in = is_substrate_env(substrate)
        allow_out = is_substrate_env(real_session)
        self.assertTrue(allow_in)
        self.assertFalse(allow_out)


# ---------------------------------------------------------------------------
# Clipboard save/restore (Requirements 7.2, 20.2)
# ---------------------------------------------------------------------------


class FakeClipStore:
    """An in-memory stand-in for a clipboard backend's subprocess.run."""

    def __init__(self, initial: bytes = b"") -> None:
        self.contents = initial
        self.calls: list[list[str]] = []

    def run(self, cmd, *, capture_output=False, check=False, timeout=None, input=None):  # noqa: A002
        self.calls.append(list(cmd))
        # Read command: any cmd ending in -o / --no-newline returns contents.
        if "-o" in cmd or "--no-newline" in cmd:
            return subprocess.CompletedProcess(cmd, 0, stdout=self.contents, stderr=b"")
        # Write command: stash the input bytes.
        if input is not None:
            self.contents = input
        return subprocess.CompletedProcess(cmd, 0, stdout=b"", stderr=b"")


class ClipboardTests(unittest.TestCase):
    X11_TOOLS = ClipboardTools(
        backend="x11",
        read_cmd=["xclip", "-selection", "clipboard", "-o"],
        write_cmd=["xclip", "-selection", "clipboard", "-i"],
    )

    def test_save_then_restore_roundtrip(self) -> None:
        store = FakeClipStore(initial=b"user's important clipboard")
        saved = save_clipboard(self.X11_TOOLS, runner=store.run)
        self.assertEqual(saved, b"user's important clipboard")

        # Simulate a test mutating the clipboard mid-run.
        store.contents = b"clobbered-by-test"
        ok = restore_clipboard(saved, self.X11_TOOLS, runner=store.run)
        self.assertTrue(ok)
        self.assertEqual(store.contents, b"user's important clipboard")

    def test_restore_none_is_noop(self) -> None:
        store = FakeClipStore(initial=b"unchanged")
        self.assertFalse(restore_clipboard(None, self.X11_TOOLS, runner=store.run))
        self.assertEqual(store.contents, b"unchanged")

    def test_save_returns_none_when_no_backend(self) -> None:
        none_tools = ClipboardTools(backend="none", read_cmd=None, write_cmd=None)
        self.assertIsNone(save_clipboard(none_tools, runner=lambda *a, **k: None))

    def test_save_never_raises_on_backend_failure(self) -> None:
        def boom(*_a, **_k):
            raise OSError("clipboard tool crashed")

        self.assertIsNone(save_clipboard(self.X11_TOOLS, runner=boom))
        self.assertFalse(restore_clipboard(b"data", self.X11_TOOLS, runner=boom))

    def test_detect_prefers_wayland_then_x11_then_none(self) -> None:
        present = {"wl-paste", "wl-copy", "xclip"}

        def which(name: str) -> str | None:
            return f"/usr/bin/{name}" if name in present else None

        wl = detect_clipboard_tools({"WAYLAND_DISPLAY": "wayland-0"}, which=which)
        self.assertEqual(wl.backend, "wayland")

        x = detect_clipboard_tools({"DISPLAY": ":99"}, which=which)
        self.assertEqual(x.backend, "x11")

        none = detect_clipboard_tools({}, which=lambda _n: None)
        self.assertEqual(none.backend, "none")


# ---------------------------------------------------------------------------
# Launcher script
# ---------------------------------------------------------------------------


class LauncherScriptTests(unittest.TestCase):
    def test_launcher_exists_and_executable(self) -> None:
        self.assertTrue(LAUNCHER.is_file(), f"missing {LAUNCHER}")
        self.assertTrue(os.access(LAUNCHER, os.X_OK), "launcher not executable")

    def test_launcher_refuses_real_display(self) -> None:
        # Refuse :0 — must never reuse the real desktop session.
        result = subprocess.run(
            ["bash", str(LAUNCHER), "--display", ":0", "--mode", "xvfb", "--", "true"],
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REFUSING", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
