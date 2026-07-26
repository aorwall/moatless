# PORT-1 — Inject binding and connection repositories into handlers

- **Priority:** P2
- **Status:** Ready for agent
- **Blocked by:** Schedule after ING-2 and ING-3 to reduce handler conflicts.

## What to build

Expose handler-facing task-binding and adapter-connection ports through
`AppState`. Wire the PostgreSQL implementations in composition and replace
direct `PgTaskBindingRepository` and `PgAdapterConnectionRepository`
construction across handlers.

## Acceptance criteria

- [ ] `AppState` exposes the two handler-facing ports.
- [ ] Production composition supplies the PostgreSQL implementations.
- [ ] Handlers depend on the ports, not concrete repository construction.
- [ ] Concrete construction remains in composition, repository, or service
      layers.
- [ ] Mocks/fakes are introduced only where tests exercise them.

## Verification

- Handler/service tests using exercised fake ports.
- Task-connection and adapter integration suites.
