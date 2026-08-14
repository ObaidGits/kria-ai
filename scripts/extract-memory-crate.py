#!/usr/bin/env python3
"""Move `kria-core/src/memory` into the `kria-memory` crate.

Run once:  python3 scripts/extract-memory-crate.py

# What has to happen atomically

1. `git mv` the module so history follows the files.
2. `memory/mod.rs` becomes the crate root `lib.rs`.
3. Inside the moved code, `crate::memory::X` becomes `crate::X` — the module is now
   the crate root, so the old path no longer exists.
4. `crate::llm::{ChatMessage, LlmBackend}` is replaced by the inverted trait.

Doing these in one pass matters: any one alone leaves the tree unbuildable, and a
half-moved 105,000-line module is harder to reason about than either end state.
"""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CORE = ROOT / "crates/kria-core/src"
DEST = ROOT / "crates/kria-memory/src"
SRC = CORE / "memory"


def run(*args: str) -> None:
    subprocess.run(args, cwd=ROOT, check=True)


if not SRC.is_dir():
    print(f"nothing to move: {SRC} does not exist (already extracted?)")
    sys.exit(0)

DEST.mkdir(parents=True, exist_ok=True)

print("1/4 moving files with git mv ...")
for child in sorted(SRC.iterdir()):
    target = DEST / child.name
    run("git", "mv", str(child.relative_to(ROOT)), str(target.relative_to(ROOT)))
SRC.rmdir()

mod_rs = DEST / "mod.rs"
lib_rs = DEST / "lib.rs"
if mod_rs.exists():
    print("2/4 mod.rs -> lib.rs ...")
    run("git", "mv", str(mod_rs.relative_to(ROOT)), str(lib_rs.relative_to(ROOT)))

print("3/4 rewriting internal paths ...")
rewrites = 0
llm_files = []
for path in sorted(DEST.rglob("*.rs")):
    text = original = path.read_text(encoding="utf-8")

    # `crate::memory::X` -> `crate::X` (this IS the crate now).
    text = text.replace("crate::memory::", "crate::")
    # A bare `crate::memory` with no trailing path refers to the crate root.
    text = re.sub(r"crate::memory\b(?!::)", "crate", text)

    if "crate::llm" in text:
        llm_files.append(path.name)
        text = text.replace(
            "use crate::llm::{ChatMessage, LlmBackend};",
            "use crate::llm_seam::SemanticExtractionLlm;",
        )

    if text != original:
        path.write_text(text, encoding="utf-8")
        rewrites += 1

print(f"      rewrote {rewrites} file(s); llm references in: {llm_files or 'none'}")

print("4/4 declaring llm_seam in the crate root ...")
lib = lib_rs.read_text(encoding="utf-8")
if "pub mod llm_seam;" not in lib:
    lines = lib.split("\n")
    insert = 0
    while insert < len(lines) and (
        lines[insert].startswith("//!") or lines[insert].strip() == ""
    ):
        insert += 1
    lines.insert(
        insert,
        "/// The one language-model capability this crate needs, declared here so the\n"
        "/// crate does not depend on the application's LLM stack.\npub mod llm_seam;\n",
    )
    lib_rs.write_text("\n".join(lines), encoding="utf-8")

print("done. Next: workspace member + core re-export.")
