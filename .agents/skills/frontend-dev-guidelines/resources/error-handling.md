# Error handling

Use [useApiError](../../../../apps/frontend/src/lib/hooks/use-api-error.ts) to format
errors. It returns `getErrorMessage`, `getValidationErrors`, `isValidationError`,
and `formatErrorForDisplay`. Toasts use `sonner`; the hook does not show them.

[ApiError](../../../../apps/frontend/src/lib/api/client.ts) exposes `status` and
`apiError`. Not every rejected fetcher is an `ApiError`; `unwrapResponse` can throw
an Error carrying a status. Narrow unknown errors before reading properties.

Read [global query/mutation handling](../../../../apps/frontend/src/lib/api/query-client.ts)
before adding local toasts, retries, or login redirects. Give a form its validation
feedback, but avoid duplicate global and local notifications. Session health owns
server-down detection; an individual failed query is not a health probe.

For live updates, use the existing
[event stream provider](../../../../apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx)
and its exported hooks. Preserve teardown and cache ownership when subscribing.
