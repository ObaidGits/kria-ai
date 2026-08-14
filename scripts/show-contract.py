"""Print the frozen contract for a named set of tools."""

import json
import pathlib
import sys

ROOT = pathlib.Path("/media/obaid/SSD/KRIA")
ops = json.loads(
    (ROOT / ".kiro/specs/linux-os-control-production/operation-contracts.json").read_text()
)["operations"]

wanted = set(sys.argv[1:])
for op in ops:
    if op["toolName"] not in wanted:
        continue
    print("=" * 70)
    print(op["toolName"], "  task", op.get("taskId"), " req", op.get("requirementId"))
    print("  target       :", op.get("target"))
    print("  risk         :", op.get("riskFunctionId"), op.get("riskRules"))
    print("  verification :", op.get("verificationClass"))
    print("  rollback     :", op.get("rollbackClaim"))
    print("  redaction    :", op.get("redactionProfile"))
    print("  resources    :", json.dumps(op.get("canonicalResourceDerivation"))[:300])
    print("  input        :", json.dumps(op.get("inputSchema"))[:600])
    print("  output       :", json.dumps(op.get("outputSchema"))[:400])
    print("  provider     :", json.dumps(op.get("providerOperation"))[:300])
