# ASYNC-1 — Move sandbox initialization filesystem work off Tokio workers

- **Priority:** P2
- **Status:** Ready for agent
- **Blocked by:** None

## What to build

Keep the existing synchronous filesystem helpers unit-testable, but call
directory setup, recursive plugin-skill staging, Claude/Codex compatibility
mirroring, and repository workspace bridging through bounded `spawn_blocking`
boundaries during async initialization.

## Acceptance criteria

- [ ] Recursive and cleanup filesystem work does not run on a Tokio worker.
- [ ] Blocking-task panics and join failures become initialization errors with
      context.
- [ ] Initialization ordering and existing filesystem behavior remain stable.
- [ ] The redundant runtime environment mutation in directory setup is removed
      or moved to a safe pre-runtime boundary; the container entrypoint remains
      the source where it already exports the value.

## Verification

- Existing filesystem behavior tests.
- Injected blocking-task error/panic propagates as an initialization failure.
- Initialization order remains unchanged.
