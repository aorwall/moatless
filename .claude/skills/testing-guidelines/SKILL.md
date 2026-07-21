---
name: testing-guidelines
description: Vitest testing conventions for the Moatless Vibe frontend and TS packages — tier classification, strategic mocking, naming, coverage. Use when writing or reviewing frontend/TS tests.
---

# Testing Guidelines

Vitest + React Testing Library for `apps/frontend` and TS packages. Backend is Rust: `cargo test -p <crate>` with the conventions in `.claude/rules/`. E2E (Rust integration tiers, Playwright, cluster suite) is owned by the **e2e-testing** skill.

## Resources

| Topic | File |
|---|---|
| Testing philosophy (what to test / skip) | [testing-philosophy.md](resources/testing-philosophy.md) |
| Tier classification (where coverage goes) | [tier-classification.md](resources/tier-classification.md) |
| Strategic mocking (what to mock / not) | [strategic-mocking.md](resources/strategic-mocking.md) |
| Component/hook test how-to | [frontend testing.md](../frontend-dev-guidelines/resources/testing.md) |
| Worked examples | [complete-examples.md](resources/complete-examples.md) |

## Repo facts

- **Naming**: unit tests are `*.unit.test.tsx` (dominant convention; plain `*.test.ts` also exists), in `__tests__/` next to the code. Integration: `*.integration.test.ts`.
- **E2E is Playwright**, not Vitest: `bun run test:e2e` (API suite), `test:e2e:ui` — under `tests/`, not `src/`.
- **Coverage**: `@vitest/coverage-v8` via `bun run test:coverage`.
- **Render helper**: `render` from `@/test-utils/render` (wraps QueryClient + mock providers).

## Commands

```bash
bun run test              # all Vitest (excludes E2E)
bun run test:unit         # fastest
bun run test:integration
bun run test:coverage
bun run test task-card    # single file/pattern
```

## Principles

- **Tiers drive effort**: Tier 1 (state machines, business logic) 80–90% coverage · Tier 2 (conditional rendering, data fetching) 50–60% · Tier 3 (presentational) 0%. Details: tier-classification.md.
- **Mock external dependencies only** (APIs, browser APIs, third-party) — never your own application code. Details: strategic-mocking.md.
- **Assert user-visible behavior**, not internal state. Speed budgets: unit <200ms, integration <2s.
