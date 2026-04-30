import { z } from 'zod'

/**
 * Search params shared across the task detail routes (`/tasks/$task-id/` and
 * `/tasks/$task-id/headless`). Both routes accept the same shape so that
 * route-agnostic hooks (e.g. `useTaskView`) can read/write search state from
 * either one.
 *
 * @property view - Current view: 'chat', 'diff', or a server name (e.g. 'files', 'preview', 'storybook')
 * @property file - Selected file path in file explorer
 * @property path - Current path in preview (e.g., /about)
 * @property viewport - Preview viewport mode: mobile/tablet/desktop
 */
export const taskSearchSchema = z.object({
  view: z.string().optional(),
  file: z.string().optional(),
  path: z.string().optional(),
  viewport: z.enum(['mobile', 'tablet', 'desktop']).optional(),
})

export type TaskSearch = z.infer<typeof taskSearchSchema>
