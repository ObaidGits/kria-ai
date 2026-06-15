# Scripts

The `scripts/` directory is for operational setup/runtime helpers. Test and eval
orchestration lives under the centralized testing spine:

```bash
./testing/run.sh all --profile ci --fail-fast
./testing/run.sh n8n --profile ci --fail-fast
./testing/run.sh n8n --tag prompt_e2e --include-live --include-slow
./testing/run.sh eval_engine
./testing/run.sh release_live --include-live --include-destructive --include-slow
```

## Operational Scripts Kept Here

Examples:

- `setup.sh`, `setup.ps1`
- `setup_python.sh`
- `setup_google_workspace.sh`
- `setup-kria-net.sh`
- `download_models.py`
- `detect_hardware.sh`
- `fix-inotify-limit.sh`
- `docker-entrypoint.sh`
- `uninstall.sh`, `uninstall.ps1`
- `provision_n8n_stage2_6_workflows.sh`
- `gui_cognition_test_substrate.sh` — stands up the isolated **TestSubstrate**
  (nested compositor or Xvfb for CI) with a scratch `HOME`, sample files, and
  clipboard save/restore, and marks the process (`KRIA_GUI_TEST_SUBSTRATE=1`) so
  the GUI Cognition backend gates auto-approval fixtures to the substrate only.
  See `docs/gui_cognition_test_substrate.md`.

Do not add new test/eval entrypoints here. Add test command implementations
under `testing/suites/<suite>/commands` and register them in that suite's
manifest.
