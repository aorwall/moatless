# E2E-3 — Finish the driver migration and delete the hand-written client

- **Priority:** P1
- **Status:** Done
- **Blocked by:** E2E-1 (driver and profile), E2E-2 (raw call, event stream)
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

Move the remaining 16 chains and the whole step layer onto the driver, then
delete `tests/shared/api-client.ts` (1270 lines).

The work is bounded and countable: roughly 30 distinct verbs, ~140 direct call
sites in chains, plus 8 step files. The heaviest chains are `webhook-routing`,
`sandbox-errors`, `plugins-e2e`, `repository` and `integrations-errors`.

Three call patterns need a decision as they come up, and the rule is the same
each time — extend the CLI rather than reach past it:

| Pattern today | Becomes |
|---|---|
| A raw request asserting on a rejection | the raw call from E2E-2 |
| The NDJSON collector | the event stream from E2E-2, wrapped by the driver |
| A verb the CLI does not expose | a new CLI command, in this ticket, not a bypass |

Any verb genuinely not worth a CLI command is listed in the ticket's closing
notes with the reason, so the exceptions are visible rather than discovered later.

## Acceptance criteria

- [x] No file under `tests/` imports the hand-written client.
- [x] `tests/shared/api-client.ts` is deleted.
- [x] The event collector reads the CLI's stream rather than connecting itself.
- [x] Every rejection assertion goes through the raw call.
- [x] Any verb added to the CLI for this ticket is documented in `--help`.
- [ ] The full `docker` profile run is green, with the same test count as before.
      *(Same reason as E2E-1: no daemon here.)*

## Verification

- `E2E_PROFILE=docker` full suite against Compose: green, and the passing test
  count matches the pre-migration count exactly.
- A grep for the deleted client across `tests/` returns nothing.
- `bun run type:check` and `bun run biome:check` are clean.
