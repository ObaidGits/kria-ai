from __future__ import annotations

from typing import Any

from testing.harness.models import RunContext, Scenario


def run_cleanup_hooks(scenario: Scenario, context: RunContext) -> dict[str, Any]:
    if not scenario.cleanup:
        return {"status": "not_required", "actions": []}
    actions = []
    failed = False
    for hook in scenario.cleanup:
        kind = hook.get("kind", "unknown")
        # v1 intentionally records hook intent only. Destructive resource cleanup
        # needs a concrete suite implementation before execution is allowed.
        if kind == "record_only":
            actions.append({"kind": kind, "status": "passed", "message": hook.get("message", "")})
        else:
            failed = True
            actions.append(
                {
                    "kind": kind,
                    "status": "failed",
                    "message": "cleanup hook kind is not implemented in the spine v1",
                }
            )
    return {"status": "failed" if failed else "passed", "actions": actions}

