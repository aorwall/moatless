# Test placement

| Behavior | Boundary |
|---|---|
| Pure calculation, parser, transition | Unit test beside its owner |
| Hook or component interaction | Vitest with Testing Library and real feature logic |
| Several application modules plus external I/O | Integration test with the external boundary controlled |
| Product lifecycle through the shipped CLI or browser | Playwright suite under `tests/` |
| Repository script process and filesystem behavior | Root `scripts/__tests__`, run by `bun run test:scripts` |

State machines and business decisions merit the most coverage. Conditional UI
needs its distinct visible outcomes covered. Static presentation usually needs
no unit test. Coverage percentages are signals, not acceptance criteria; a high
percentage does not prove a missing error path works.
