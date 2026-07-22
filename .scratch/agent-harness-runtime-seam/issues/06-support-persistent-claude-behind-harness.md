# 06 — Support persistent Claude execution behind the same seam

**What to build:** Preserve Claude’s persistent transport, mid-Run input injection, process fingerprint replacement, and harness-internal work entirely behind the shared Agent harness interface. The coordinator consumes the same signals as the subprocess path and learns no tmux-specific behavior.

**Blocked by:** 05 — Run Claude subprocess Turns through the Agent harness

**Status:** ready-for-agent

- [ ] A compatible input can be injected into the active persistent Agent process.
- [ ] Injected input emits input acceptance without another execution-start signal and is acknowledged against the current Run.
- [ ] Incompatible model, mode, system prompt, Session request, or explicit fresh start replaces the process safely and resumes or starts the intended Session.
- [ ] Process replacement fully terminates the old Agent process before starting the replacement, preserving the at-most-one-process invariant.
- [ ] Harness-internal wakeup or Monitor activity emits execution start before its first source event.
- [ ] Harness-internal execution creates no pending-Message acknowledgement.
- [ ] Background activity and deferred work continue to protect active Sandboxes from idle reaping.
- [ ] Persistent-process termination, stream failure, and cancellation each produce exactly one terminal outcome.
- [ ] Pending input remains ordered while process replacement or an active execution is in progress.
- [ ] The coordinator contains no persistent-transport or tmux branch.
- [ ] Scripted tests cover injection, incompatible replacement, internal execution, cancellation, EOF, and queued input.
- [ ] A focused persistent-Claude E2E case exists for final integrated verification of actual transport launch, injection, and Session continuity.
- [ ] No shared harness or coordinator file becomes a repository for Claude-native payload mapping.
- [ ] Sandbox clippy passes with warnings denied.
- [ ] The full Sandbox crate test suite passes.
