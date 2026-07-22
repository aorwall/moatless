# 03 — Establish the Agent harness command/signal seam

**What to build:** Introduce the concrete Agent harness module and prove one complete mock-driven execution from submitted input through Session binding, Run creation, acknowledgement, ordered source publication, and terminal completion. Existing Claude and Codex production paths remain available until later migration tickets.

**Blocked by:** None — can start immediately

**Status:** ready-for-agent

- [ ] The harness exposes a small operational interface for submitting input, cancelling active execution, and receiving the next normalized signal.
- [ ] Construction fixes one harness implementation for the Sandbox lifecycle.
- [ ] Commands and signals use bounded delivery or equivalent explicit backpressure; nothing is silently dropped.
- [ ] The shared signal vocabulary covers execution start, input acceptance, one-block source events, activity, terminal outcome, and pre-start input rejection.
- [ ] A started execution always produces exactly one terminal outcome.
- [ ] No source event is emitted before execution start.
- [ ] The harness owns no Run identity, pending-Message acknowledgement, retry policy, backend publisher, or Message assembly dependency.
- [ ] A scripted mock Agent process can emit Session starts, source blocks, failures, EOF, and terminal results deterministically.
- [ ] A harness-neutral coordinator test mints the Run after execution start and acknowledges input only after input acceptance.
- [ ] Tests cover the new-execution ordering contract, cancellation idempotency, pre-start rejection, early-output buffering, backpressure, and closed channels.
- [ ] Private process seams have real production and mock adapter roles; no public universal harness trait is introduced.
- [ ] Module roots expose only the small interface, while ordering, Session rules, errors, and test support have focused private ownership.
- [ ] Existing production Agent behavior is unchanged when the new seam is not selected.
- [ ] Sandbox clippy passes with warnings denied.
- [ ] The full Sandbox crate test suite passes.
