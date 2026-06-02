# KRIA Security And Audit Suite

Central registration for security, policy, audit, and dangerous-action tests.

```bash
./testing/run.sh security_audit
./testing/run.sh security_audit --include-live
./testing/run.sh security_audit --include-destructive --include-slow
```

Safe audit tests can run by default. Live or destructive checks stay behind
explicit flags.

