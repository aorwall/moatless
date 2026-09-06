# Data fetching

Use generated fetchers and models from `@moatless/api-client`, wrapped with
`unwrapResponse` from `@/lib/api/client`. Feature hooks own query options and
mutations. Start with [task queries](../../../../apps/frontend/src/features/tasks/hooks/use-task-queries.ts)
and [query keys](../../../../apps/frontend/src/lib/api/query-keys.ts).

- `queryKeys.tasks.all` is an array constant, not a function. Detail/list keys
  use the helpers and filters declared in that module.
- Task lists return an object containing `items`; pagination uses `limit` and
  `offset`. Do not treat the response as a task array or send `page`.
- Create requests use `name`; responses identify tasks with `taskId`. Task status
  is `open | closed | error`; agent state is a separate field.
- Invalidate the narrowest affected key and keep event-driven cache updates in
  the existing event integration. Do not invent another event bus.

Read [query-client.ts](../../../../apps/frontend/src/lib/api/query-client.ts) before
changing retries or error behavior. It owns conditional retries, auth redirects,
mutation fallback toasts, and query metadata. Do not copy those defaults into
feature hooks. Regenerate the client with `bun run generate:api` for schema changes.
