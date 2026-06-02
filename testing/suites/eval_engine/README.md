# KRIA Eval Engine Suite

Central registration for `crates/kria-eval`.

```bash
./testing/run.sh eval_engine
./testing/run.sh eval_engine --include-slow
```

Phase 2 registers package-level eval-engine tests without moving the eval crate.
The package command is marked `slow` because it compiles and exercises the eval
engine broadly.

GUI eval command implementations live in `testing/suites/eval_engine/commands`.
Use `./testing/run.sh eval_engine ...` instead of adding direct scripts under
`scripts/`.
