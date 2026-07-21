/**
 * Task-page view identifiers used by the `?view=` URL param.
 *
 * Server views use the server name directly (e.g. `preview`, `storybook`);
 * these constants name the fixed, always-available views. Defined here — not in
 * a component file — so the vocabulary lives in one place.
 *
 * NOTE: `features/preview/components/view-tabs.tsx` still declares its own
 * `SANDBOX_VIEW` literal because `features/preview` may not import
 * `features/tasks`. Both resolve to the same string; unify once `ViewTabs`
 * moves into the tasks feature (see the composability plan's rename table).
 */
export const SANDBOX_VIEW = 'sandbox'
