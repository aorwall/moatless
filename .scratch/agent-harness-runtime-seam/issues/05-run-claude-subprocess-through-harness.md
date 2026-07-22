# 05 — Run Claude subprocess Turns through the Agent harness

**What to build:** Execute complete Claude subprocess Turns through the shared harness seam while preserving Session behavior, interactive prompts, cancellation, activity, and user-visible output. Claude emits canonical per-block source events and the coordinator handles lifecycle policy without Claude-specific branches.

**Blocked by:** 02 — Expand backend Message assembly for per-block source events; 03 — Establish the Agent harness command/signal seam; 04 — Fix the Agent harness at Sandbox construction

**Status:** ready-for-agent

- [ ] A Claude subprocess can start a fresh Session, resume a known Session, and continue the latest recoverable Session through the harness.
- [ ] Execution start is signalled only after the process is ready and the real Session identity is known.
- [ ] Input acceptance occurs only after Claude accepts the input, allowing correct Message acknowledgement.
- [ ] Claude assistant envelopes produce one ordered canonical source event per native content block.
- [ ] Repeated usage is forwarded for backend admission rather than removed or coalesced in this path.
- [ ] Text, thinking, tool calls, tool results, model data, parent tool identity, and bounded native diagnostics remain observable.
- [ ] Ask-user and plan-approval interactions continue to pause and resume the active execution correctly.
- [ ] Tool-progress activity remains observable for liveness without becoming Message content.
- [ ] Cancellation terminates the Claude process tree and produces exactly one cancelled terminal outcome.
- [ ] Authentication, rate limit, unavailable Session, malformed protocol, and unexpected process exit become stable harness failure classes with Session impact.
- [ ] Resume failure does not silently start fresh; the coordinator explicitly drives the existing context-loss behavior.
- [ ] Model, mode, system prompt, environment, and permission configuration retain current behavior.
- [ ] Scripted contract tests cover happy path, prompts, resume, failure, cancellation, and early output.
- [ ] A focused real Claude subprocess E2E case exists for the final integrated verification of actual CLI launch and protocol compatibility.
- [ ] Sandbox and backend clippy pass with warnings denied.
- [ ] Relevant Sandbox, backend, and database integration tests pass.
