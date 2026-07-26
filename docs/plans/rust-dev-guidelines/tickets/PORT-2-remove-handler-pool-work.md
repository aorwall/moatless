# PORT-2 — Remove remaining direct handler pool work

- **Priority:** P2
- **Status:** Ready for agent
- **Blocked by:** PORT-1

## What to build

Move remaining handler-owned persistence into narrowly owned repositories or
services: repository soft-delete cascade SQL, Linear token/API construction,
Slack identity parsing/deduplication, and task-binding batch enrichment.

## Acceptance criteria

- [ ] Each operation has one repository/service owner.
- [ ] Handler code no longer passes the raw pool into domain/adapter behavior.
- [ ] Any remaining direct handler pool access is documented in a narrow
      authentication or infrastructure allowlist.
- [ ] Public API behavior remains unchanged.

## Verification

- Repository deletion cascade.
- Slack identity resolution and deduplication.
- Linear token resolution.
- Task-list binding enrichment.
