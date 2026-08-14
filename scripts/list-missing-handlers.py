"""List frozen manifest tools that have no registered handler, grouped by phase+task."""

import json
import pathlib
import re
from collections import defaultdict

ROOT = pathlib.Path("/media/obaid/SSD/KRIA")

spec = ROOT / ".kiro/specs/linux-os-control-production/operation-contracts.json"
ops = json.loads(spec.read_text())["operations"]

meta = {}
for op in ops:
    meta[op["toolName"]] = {
        "task": op.get("taskId", "?"),
        "phase": op.get("phase", "?"),
        "req": op.get("requirementId", "?"),
        "risk": op.get("riskFunctionId", "?"),
        "verification": op.get("verificationClass", "?"),
        "rollback": op.get("rollbackClaim", "?"),
        "provider": op.get("providerOperation", "?"),
    }

registered = set()
pattern = re.compile(r'name:\s*"([a-z0-9_]+)"\.into\(\)')
for path in (ROOT / "crates/kria-core/src/tools").rglob("*.rs"):
    text = path.read_text(encoding="utf-8", errors="ignore")
    registered.update(pattern.findall(text))

missing = {t: m for t, m in meta.items() if t not in registered}
print(f"manifest={len(meta)}  have_handler={len(meta) - len(missing)}  missing={len(missing)}")
print()

by_task = defaultdict(list)
for tool, m in sorted(missing.items()):
    by_task[(m["phase"], m["task"])].append(tool)

for key in sorted(by_task):
    phase, task = key
    tools = by_task[key]
    print(f"[{phase}] task {task}  ({len(tools)})")
    for tool in tools:
        m = missing[tool]
        print(f"    {tool:34s} {m['provider']}")
