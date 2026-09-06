---
name: testing-guidelines
description: "Write and review Vitest tests for the Moatless frontend, TypeScript packages, and repository scripts. Use for test placement, component or hook tests, mocking boundaries, and regression coverage."
---

# TypeScript testing

Test observable behavior where it can fail. Frontend/component tests use Vitest
and React Testing Library; E2E uses [e2e-testing](../e2e-testing/SKILL.md), and Rust
uses [rust-dev-guidelines](../rust-dev-guidelines/SKILL.md).

- [Test selection](resources/testing-philosophy.md): when coverage earns its cost.
- [Placement](resources/tier-classification.md): choose the smallest useful boundary.
- [Mocking](resources/strategic-mocking.md): keep application behavior under test real.
- [Examples](resources/complete-examples.md): maintained component and hook tests.

## Commands

From the repository root:

```bash
bun run test
bun run test:unit
bun run test:integration
bun run test:coverage
bun run test:scripts
```

For one frontend file, run Vitest inside that workspace:

```bash
cd apps/frontend
bun run test src/features/tasks/utils/__tests__/format-duration.unit.test.ts
```

Root `test` goes through Turbo; an extra bare pattern is interpreted as a Turbo
task, not a Vitest filter. Name unit tests `*.unit.test.ts(x)` in nearby
`__tests__` directories; existing `*.test.ts` files are also supported.

## Component and hook tests

Use `render` and `createTestQueryClient` from
[the repository helper](../../../apps/frontend/src/test-utils/render.tsx).
Create fresh query state per test. Drive accessible UI controls and await the
observable result; do not use arbitrary sleeps or assert hook internals.
For asynchronous hooks, use Testing Library's `renderHook` and `waitFor` with
the providers the hook actually needs. Clean up timers, network servers, and
processes even after a failed assertion.
