# Test placement

| Behavior | Boundary |
|---|---|
| Pure calculation, parser, transition | Unit test beside its owner |
| Several application modules plus external I/O | Integration test with the external boundary controlled |
| Product lifecycle through the shipped CLI | Playwright suite under `tests/` |
| Repository script process and filesystem behavior | Root `scripts/__tests__`, run by `bun run test:scripts` |

State machines and business decisions merit the most coverage. Coverage
percentages are signals, not acceptance criteria; a high percentage does not
prove a missing error path works.
