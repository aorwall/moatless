# Archived UI Tests

These files are **non-functional legacy tests** preserved as inspiration for writing new browser-based UI tests.

They were moved here from `tests/e2e/` during the test restructuring. Import paths and APIs they reference may no longer exist. Do not attempt to run them as-is.

## What's here

- `smoke.spec.ts` — Basic smoke tests (app loads, API endpoints respond, SSE works)
- `ui-task-complete-workflow.spec.ts` — Full UI workflow: create repo, select it in dropdown, enter task message, submit, monitor agent execution, verify file creation
- `helpers/` — Server startup/teardown and test utilities used by the workflow test
