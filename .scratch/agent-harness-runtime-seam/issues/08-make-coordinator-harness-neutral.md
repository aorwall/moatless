# 08 — Make the coordinator fully harness-neutral

**What to build:** Consolidate Run and Turn orchestration, Message acknowledgement, retry policy, credential recovery, cancellation, liveness, and backend publishing into one coordinator that reacts only to shared harness signals. Adding a future harness must not change this lifecycle policy.

**Blocked by:** 06 — Support persistent Claude execution behind the same seam; 07 — Run Codex one-shot Turns through the Agent harness

**Status:** ready-for-agent

- [ ] The coordinator contains no Claude, Codex, tmux, one-shot, or native-protocol branch.
- [ ] A Run is minted and its Turn activated only after execution start with a known Session.
- [ ] Pending Messages are acknowledged only after input acceptance and are associated with the correct Session and current Run.
- [ ] Input joining an active execution is acknowledged against the current Run; queued input is acknowledged against the next Run.
- [ ] Harness-internal execution creates a Run without acknowledging a pending Message.
- [ ] Source events and activity are published with the coordinator-owned Run identity.
- [ ] Every terminal harness outcome closes the Run and Turn exactly once.
- [ ] Stable harness failure facts drive retry count, exponential backoff, circuit breaking, credential recovery, explicit fresh-Session fallback, or terminal failure.
- [ ] The harness never receives retry policy, acknowledgement, Task state, or backend publisher dependencies.
- [ ] Cancellation and duplicate notifications remain idempotent across coordinator and harness.
- [ ] Coordinator tests use a scripted harness and in-memory backend adapter rather than real CLIs.
- [ ] Tests cover started/accepted ordering, injected input, queued input, internal execution, every error class, retry exhaustion, dropped acknowledgements, and duplicate terminal signals.
- [ ] Coordinator responsibilities are split by owned concept; the orchestration loop does not absorb delivery, retry, or native mapping logic.
- [ ] Sandbox clippy passes with warnings denied.
- [ ] The full Sandbox crate and relevant integration suites pass.
