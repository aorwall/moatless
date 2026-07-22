# 04 — Fix the Agent harness at Sandbox construction

**What to build:** Resolve the Task’s selected Agent harness once when constructing its Sandbox harness instance. Pending Messages may configure a Turn but cannot switch the harness, and an unsupported selection fails clearly instead of silently running Claude.

**Blocked by:** 03 — Establish the Agent harness command/signal seam

**Status:** ready-for-agent

- [ ] The Task remains the authoritative source of its selected Agent harness.
- [ ] Sandbox construction receives or resolves that selection before pending input is dispatched.
- [ ] One harness instance is created for the Sandbox lifecycle and its selection cannot change.
- [ ] Pending Messages no longer control which harness implementation executes them.
- [ ] Legacy per-Message selection data is tolerated only as migration input and cannot override the Task selection.
- [ ] Unsupported or unknown harness values produce a classified startup/configuration failure rather than defaulting to Claude.
- [ ] Claude transport, model, and mode variants remain configuration of the Claude harness rather than separate harnesses.
- [ ] Existing Tasks retain their current Claude or Codex selection without user migration work.
- [ ] Restart and repeated-notification tests prove the selection remains fixed.
- [ ] Backend-to-Sandbox integration tests prove the Task selection reaches harness construction correctly.
- [ ] Sandbox and backend clippy pass with warnings denied.
- [ ] Relevant Sandbox, backend, and integration test suites pass.
