# 01 — Delete the abandoned Agent executor

**What to build:** Remove the uncompiled duplicate Agent execution architecture so maintainers and coding Agents see one authoritative path before the new harness seam is introduced. Preserve any still-valuable behavioral expectations by moving them to tests of active code rather than retaining dead implementation.

**Blocked by:** None — can start immediately

**Status:** ready-for-agent

- [ ] The abandoned executor abstraction and duplicate Claude implementation are removed.
- [ ] No active production module references `AgentExecutor`, `ClaudeCodeExecutor`, or the retired orchestrator entry point.
- [ ] Useful batching, lifecycle, or error expectations found only in the retired tests are either proven by active tests or deliberately documented as obsolete.
- [ ] No replacement wrapper or second execution path is introduced.
- [ ] The Sandbox crate builds without dead-code allowances added for this cleanup.
- [ ] Sandbox clippy passes with warnings denied.
- [ ] The full Sandbox crate test suite passes.
