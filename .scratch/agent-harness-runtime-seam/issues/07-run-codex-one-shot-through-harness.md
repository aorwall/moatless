# 07 — Run Codex one-shot Turns through the Agent harness

**What to build:** Execute complete Codex Turns through the shared harness seam while hiding its one-process-per-Turn strategy, thread resume, queueing, native event translation, and collision-safe identity. Users retain all current Codex output and recovery behavior without coordinator Codex branches.

**Blocked by:** 02 — Expand backend Message assembly for per-block source events; 03 — Establish the Agent harness command/signal seam; 04 — Fix the Agent harness at Sandbox construction

**Status:** ready-for-agent

- [ ] A Codex Agent process starts one execution, reports its thread as the Session, and terminates after the Turn.
- [ ] A later execution resumes the known Codex thread using a replacement Agent process.
- [ ] Input submitted while Codex is active remains ordered and is accepted only after the current terminal outcome and next execution start.
- [ ] At most one Codex Agent process is active, including during cancellation and replacement.
- [ ] Agent messages, reasoning, commands, file changes, MCP tools, collaboration tools, web search, todo lists, errors, and unknown items retain their current observable meaning.
- [ ] Native envelopes produce ordered canonical one-block source events without Message assembly.
- [ ] Source-event identities remain collision-safe when native item numbering restarts across processes or Sandbox replacement.
- [ ] Thread, parent tool, model, usage, command result, and bounded native diagnostics are retained where available.
- [ ] Cancellation terminates the Codex process tree and produces exactly one cancelled terminal outcome.
- [ ] Authentication refresh failures, unavailable thread state, protocol errors, and unexpected process exit become stable harness failure classes with Session impact.
- [ ] Credential recovery mechanics remain hidden behind the harness while retry policy remains in the coordinator.
- [ ] Scripted tests cover fresh execution, resume, queued input, all supported item families, duplicate native IDs, credential failure, cancellation, and EOF.
- [ ] A focused real Codex E2E test proves actual CLI launch, thread resume, and protocol compatibility.
- [ ] No shared harness or coordinator file becomes a repository for Codex-native payload mapping.
- [ ] Sandbox and backend clippy pass with warnings denied.
- [ ] Relevant Sandbox, backend, and database integration tests pass.
