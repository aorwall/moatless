# Examples maintained with the application

Read these implementations for the behavior you are changing. Their types and
tests run with the application, so adapt them instead of copying a parallel tutorial.

| Change | Example |
|---|---|
| Task fetching and cache use | [use-task-queries.ts](../../../../apps/frontend/src/features/tasks/hooks/use-task-queries.ts) |
| Task creation | [use-code-task-submit.ts](../../../../apps/frontend/src/features/tasks/components/create-task/use-code-task-submit.ts) |
| Route and search state | [tasks/index.tsx](../../../../apps/frontend/src/routes/tasks/index.tsx) |
| Server error formatting | [use-api-error.ts](../../../../apps/frontend/src/lib/hooks/use-api-error.ts) |
| Configuration dialog | [sidebar-dialog stories](../../../../apps/frontend/src/lib/components/ui/__stories__/sidebar-dialog.stories.tsx) |
| Component and hook tests | [testing examples](../../testing-guidelines/resources/complete-examples.md) |

Check the generated model and fetcher signature before reusing a payload. Do not
create handwritten copies of API models to make a copied example compile.
