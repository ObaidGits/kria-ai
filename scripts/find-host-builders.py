"""Find every `with_*` builder the suites call on FakeHostOsControl, and the port type."""

import pathlib
import re

TESTS = pathlib.Path("/media/obaid/SSD/KRIA/crates/kria-core/tests")

wanted = {}
for path in sorted(TESTS.glob("os_control_*.rs")):
    text = path.read_text(encoding="utf-8", errors="ignore")
    # FakeHostOsControl::new("x") .with_a(..) .with_b(..) possibly across lines
    for m in re.finditer(r"FakeHostOsControl::new\([^)]*\)((?:\s*\.\s*with_\w+\([^;]*?\))+)", text):
        for b in re.findall(r"\.\s*(with_\w+)\(", m.group(1)):
            wanted.setdefault(b, set()).add(path.name)

print("Builders the suites call on FakeHostOsControl:")
for name in sorted(wanted):
    print(f"  {name:28s} <- {', '.join(sorted(wanted[name]))}")

print("\nPort trait candidates per domain (for the field type):")
OSC = pathlib.Path("/media/obaid/SSD/KRIA/crates/kria-core/src/os_control")
for d in sorted(p for p in OSC.iterdir() if p.is_dir()):
    mod = d / "mod.rs"
    if not mod.exists():
        continue
    traits = re.findall(r"pub trait (\w*ControlPort|\w*Port)\b", mod.read_text(encoding="utf-8", errors="ignore"))
    if traits:
        print(f"  {d.name:16s} {', '.join(sorted(set(traits)))}")
