# 10 — Contract legacy execution and verify parity

**What to build:** Remove migration compatibility and legacy execution machinery after both production harnesses and recovery run through the deep seam. Leave one navigable architecture, backend-owned Message assembly, shared contracts, and focused real-CLI verification with no public behavior regression.

**Blocked by:** 09 — Resume Sessions across Sandbox replacement

**Status:** ready-for-agent

- [ ] Legacy harness-specific actor branches and obsolete execution compatibility paths are removed.
- [ ] Sandbox-side assistant coalescing, repeated-usage tracking, and pending-buffer flushing are removed.
- [ ] The backend compatibility path for coalesced assistant events is removed once every Sandbox producer emits canonical per-block source events.
- [ ] Per-Message harness switching and silent unknown-harness fallback are removed.
- [ ] The Task’s fixed Agent harness is the only production selection path.
- [ ] Claude, Codex, and the scripted mock pass one shared harness contract suite.
- [ ] Backend reads and live upserts produce equivalent assembled Messages for text, tools, usage, and affected-parent cases.
- [ ] Subagent summary, progress, and lazy transcript behavior remain unchanged.
- [ ] Public HTTP Message contracts and client behavior remain unchanged.
- [ ] Real Claude subprocess, persistent Claude, and Codex resume smoke tests pass when their environment gates are enabled.
- [ ] Database and Sandbox integration tiers pass without migration-only test allowances.
- [ ] No production coordinator code matches on a concrete harness implementation.
- [ ] No abandoned executor, duplicate execution path, or unused compatibility abstraction remains.
- [ ] Production files are reviewed for cohesive ownership; roughly 200–300 lines is treated as a review signal, not a mechanical limit.
- [ ] Glossary and ADR wording match the final Agent harness, Agent process, Session, Run, and Message assembly behavior.
- [ ] Sandbox and backend formatting pass.
- [ ] Sandbox and backend clippy pass with warnings denied.
- [ ] Full Sandbox and backend test suites pass.
