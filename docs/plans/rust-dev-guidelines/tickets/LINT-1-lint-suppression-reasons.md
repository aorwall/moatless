# LINT-1 — Make local lint suppressions self-checking

- **Priority:** P3
- **Status:** Ready for agent
- **Blocked by:** Execute after the other Rust remediation tickets to avoid churn.

## What to build

Audit every local Rust lint suppression. Delete obsolete suppressions, use
`#[expect(lint, reason = "...")]` for exact local expectations, and reserve
`#[allow(lint, reason = "...")]` for conditional, generated, or configuration
cases where the lint may not fire. Enable
`clippy::allow_attributes_without_reason` at workspace deny level.

## Acceptance criteria

- [ ] Every remaining local suppression has a native reason.
- [ ] Exact expected hits use `expect`; conditional cases use justified `allow`.
- [ ] Obsolete suppressions are removed.
- [ ] The workspace denies unreasoned allow attributes.
- [ ] Production targets and tests remain Clippy-clean.

## Verification

- Workspace Clippy for production and test targets.
- A temporary unreasoned suppression is rejected by the workspace policy.
- An obsolete `expect` is reported as unfulfilled.
