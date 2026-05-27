# Voice Real-World Validation

This runbook defines acceptance criteria for voice behavior in realistic daily workflows.

## Validation Goals

1. Assistant-like responsiveness and interruption handling
2. Stable long-session operation
3. Safe behavior in ambiguous or risky commands
4. Trust-preserving behavior under noise, device churn, and errors

## Severity Levels

| Level | Definition | Release Impact |
|---|---|---|
| Critical | Trust/safety break | Block release |
| High | Frequent severe UX break | Fix before release |
| Medium | Noticeable friction | Track and schedule |
| Low | Minor polish | Backlog |

## Scenario Matrix

- Coding workflows
- Browser/research workflows
- Terminal and automation workflows
- File management workflows
- Interruption-heavy sessions
- Long continuous conversations
- Noisy environments
- Headphone and device-switch paths
- Dangerous edge-case prompts

## Mandatory Checks

1. Barge-in latency and stop reliability
2. Partial transcript stability and flicker bounds
3. Final transcript correctness
4. Device recovery without runtime deadlocks
5. Safety behavior for potentially destructive commands

## Execution Template

Use this per run:

```text
Date:
Scenario:
Observed Behavior:
Severity:
Action:
```

Reference architecture docs:
- `voice/overview.md`
- `voice/overview.md`
