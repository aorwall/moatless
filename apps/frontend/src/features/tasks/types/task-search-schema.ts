import { z } from 'zod'

/**
 * Search params shared across the task detail routes (`/tasks/$task-id/` and
 * `/tasks/$task-id/kiosk`). Both routes accept the same shape so that
 * route-agnostic hooks (e.g. `useTaskView`) can read/write search state from
 * either one.
 *
 * @property view - Current view: 'chat', 'diff', or a server name (e.g. 'files', 'preview', 'storybook')
 * @property file - Selected file path in file explorer
 * @property fileView - Selected file viewer mode: code/diff/preview
 * @property filesCollapsed - Whether the file tree is collapsed in the files view
 * @property path - Current path in preview (e.g., /about)
 * @property device - Preview device mode: mobile/tablet/desktop
 */
export const taskSearchSchema = z.object({
  view: z.string().optional(),
  file: z.string().optional(),
  fileView: z.enum(['code', 'diff', 'preview']).optional(),
  filesCollapsed: z
    .preprocess((value) => value === true || value === 'true', z.boolean())
    .optional(),
  path: z.string().optional(),
  device: z.enum(['mobile', 'tablet', 'desktop']).optional(),
})

export type TaskSearch = z.infer<typeof taskSearchSchema>
