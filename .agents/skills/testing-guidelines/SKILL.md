---
name: testing-guidelines
description: "Write and review Vitest tests for the Moatless TypeScript packages and repository scripts. Use for test placement, mocking boundaries, and regression coverage."
---

# TypeScript testing

Test observable behavior where it can fail. Package and script tests use Vitest;
E2E uses [e2e-testing](../e2e-testing/SKILL.md), and Rust uses
[rust-dev-guidelines](../rust-dev-guidelines/SKILL.md).

- [Test selection](resources/testing-philosophy.md): when coverage earns its cost.
- [Placement](resources/tier-classification.md): choose the smallest useful boundary.
- [Mocking](resources/strategic-mocking.md): keep application behavior under test real.

## Commands

From the repository root:

```bash
bun run test
bun run test:unit
bun run test:coverage
bun run test:scripts
```

For one package file, run Vitest inside that package:

```bash
cd packages/inspector
bunx vitest run src/__tests__/route-tracker.test.ts
```

Root `test` goes through Turbo; an extra bare pattern is interpreted as a Turbo
task, not a Vitest filter. Name unit tests `*.unit.test.ts` or `*.test.ts` in
nearby `__tests__` directories. Clean up timers, network servers, and processes
even after a failed assertion.

After bumping Vitest or a `@vitest/*` package, move the whole family to one
version in every `package.json` and run `bun run test:coverage`. `bun run test`
never loads the coverage provider, so a mismatched `@vitest/coverage-v8` passes
it and fails only under `--coverage`.
