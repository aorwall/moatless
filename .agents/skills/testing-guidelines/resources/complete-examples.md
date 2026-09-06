# Maintained test examples

| Need | Read |
|---|---|
| Rendering and query providers | [render.tsx](../../../../apps/frontend/src/test-utils/render.tsx) |
| Task creation hook | [use-code-task-submit.unit.test.ts](../../../../apps/frontend/src/features/tasks/components/create-task/__tests__/use-code-task-submit.unit.test.ts) |
| Component interaction | [feedback-dialog.unit.test.tsx](../../../../apps/frontend/src/features/tasks/components/chat/message-actions/__tests__/feedback-dialog.unit.test.tsx) |
| Query retry and auth behavior | [query-client.unit.test.ts](../../../../apps/frontend/src/lib/api/__tests__/query-client.unit.test.ts) |
| Pure utility | [format-duration.unit.test.ts](../../../../apps/frontend/src/features/tasks/utils/__tests__/format-duration.unit.test.ts) |

Use the example for setup and style, then apply the mocking rule to the boundary
your test claims to exercise. Existing tests are examples, not exemptions from
current guidance. Fixtures should use generated task models and current statuses.
