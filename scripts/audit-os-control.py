"""Audit the OS-control pipeline: what is wired, what is pending. Evidence only."""

import json
import pathlib
import re
from collections import defaultdict

ROOT = pathlib.Path("/media/obaid/SSD/KRIA")
CORE = ROOT / "crates/kria-core/src"
OSC = CORE / "os_control"

spec = ROOT / ".kiro/specs/linux-os-control-production/operation-contracts.json"
ops = json.loads(spec.read_text())["operations"]
manifest = {op["toolName"]: op for op in ops}

# 1. Tools with a handler DEFINED, and tools actually REGISTERED in the builder.
defined = set()
pattern = re.compile(r'name:\s*"([a-z0-9_]+)"\.into\(\)')
tool_files = {}
for path in (CORE / "tools").rglob("*.rs"):
    text = path.read_text(encoding="utf-8", errors="ignore")
    found = set(pattern.findall(text))
    if found:
        tool_files[path.name] = found
    defined.update(found)

registry = (CORE / "tools/registry.rs").read_text(encoding="utf-8")
registered_modules = set(re.findall(r"super::(\w+)::register", registry))

print("=" * 72)
print("1. HANDLER COVERAGE")
print("=" * 72)
missing = sorted(set(manifest) - defined)
print(f"frozen manifest tools : {len(manifest)}")
print(f"handler defined       : {len(manifest) - len(missing)}")
print(f"NO handler            : {len(missing)}")

# Which tool files hold manifest tools, and are they registered?
print("\nhandler files holding manifest tools (registered?):")
for name, tools in sorted(tool_files.items()):
    overlap = tools & set(manifest)
    if overlap:
        module = name[:-3]
        mark = "REGISTERED" if module in registered_modules else "*** NOT REGISTERED ***"
        print(f"  {module:22s} {len(overlap):3d} manifest tools   {mark}")

print("\n" + "=" * 72)
print("2. PENDING TOOLS BY PHASE / TASK")
print("=" * 72)
by_task = defaultdict(list)
for tool in missing:
    m = manifest[tool]
    by_task[(m.get("phase", "?"), m.get("taskId", "?"))].append(tool)
for key in sorted(by_task):
    print(f"[{key[0]}] task {key[1]:5s} ({len(by_task[key])}): {', '.join(by_task[key])}")

print("\n" + "=" * 72)
print("3. LIVE PROVIDER PLACEHOLDERS (fail-closed stubs)")
print("=" * 72)
total_stubs = 0
for path in sorted((OSC / "linux/providers").glob("*.rs")):
    text = path.read_text(encoding="utf-8", errors="ignore")
    n = text.count("not_yet_wired")
    if n:
        total_stubs += n
        print(f"  {path.name:26s} {n}")
print(f"  TOTAL: {total_stubs}")

print("\n" + "=" * 72)
print("4. DOMAIN TEST DOUBLES (needed to certify per-domain lifecycle)")
print("=" * 72)
domains = sorted(
    d for d in OSC.iterdir() if d.is_dir() and d.name not in {"linux", "broker"}
)
have, lack = [], []
for d in domains:
    (have if (d / "fake.rs").exists() else lack).append(d.name)
print(f"  fake.rs present ({len(have)}): {', '.join(have)}")
print(f"  fake.rs MISSING ({len(lack)}): {', '.join(lack)}")

print("\n" + "=" * 72)
print("5. PER-DOMAIN LIFECYCLE SUITES")
print("=" * 72)
suites = sorted(p.name for p in (ROOT / "crates/kria-core/tests").glob("os_control_*.rs"))
print(f"  present ({len(suites)}): {', '.join(s[:-3] for s in suites)}")

print("\n" + "=" * 72)
print("6. SPEC CHECKBOX STATE")
print("=" * 72)
tasks = (ROOT / ".kiro/specs/linux-os-control-production/tasks.md").read_text()
for mark, label in (("[x]", "complete"), ("[-]", "partial"), ("[ ]", "not started")):
    print(f"  {mark} {label:12s} {len(re.findall(re.escape('- ' + mark), tasks))}")
