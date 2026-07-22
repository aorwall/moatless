# 09 — Resume Sessions across Sandbox replacement

**What to build:** Recreate the Task’s fixed Agent harness after Sandbox replacement, restore Session state before accepting input, and resume interrupted work without confusing the harness instance, Agent process, Session, Run, or Turn lifecycles.

**Blocked by:** 08 — Make the coordinator fully harness-neutral

**Status:** ready-for-agent

- [ ] A replacement Sandbox constructs the same harness selected by the Task.
- [ ] Session-state download and restoration complete before pending input or interrupted-Run recovery reaches the harness.
- [ ] A replacement Agent process can resume the restored Claude Session or Codex thread.
- [ ] The recreated harness instance does not reuse in-memory process or execution identity from the prior Sandbox.
- [ ] Source-event identity remains collision-safe after Sandbox replacement.
- [ ] Pending input arriving during initialization is delivered once after restoration and acknowledged against the correct replacement Run.
- [ ] Interrupted-Run replay and newly pending input retain the existing precedence and do not create duplicate execution.
- [ ] Missing, corrupt, or unavailable Session state produces classified context loss rather than an implicit harness change or silent fresh Session.
- [ ] Task and Session continuity remain observable while the old harness instance and Agent process do not.
- [ ] Temporary-directory tests cover archive round trip and restored Session discovery.
- [ ] In-memory backend tests cover initialization ordering, pending input, and interrupted-Run recovery.
- [ ] Sandbox lifecycle integration tests cover replacement for both Claude and Codex harnesses.
- [ ] Sandbox and backend clippy pass with warnings denied.
- [ ] Relevant Sandbox, backend, and integration test suites pass.
