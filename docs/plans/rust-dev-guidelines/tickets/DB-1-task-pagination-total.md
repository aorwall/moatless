# DB-1 — Repair task-list pagination totals

- **Priority:** P1
- **Status:** Ready for agent
- **Blocked by:** None

## What to build

Refactor task listing so the page and total are derived from the same distinct,
filtered task relation. The total must remain available when an offset requests
an empty page.

## Acceptance criteria

- [ ] A task joined through multiple tags contributes one row and one count.
- [ ] The total reflects every active filter and access-control predicate.
- [ ] An offset beyond the final page returns an empty page with the true total.
- [ ] Existing ordering and returned task shape remain unchanged.

## Verification

- Multiple tags on one task.
- Offset beyond the end.
- Tag, status/exclusion, ACL, followed-by, and parent-relation filters.
