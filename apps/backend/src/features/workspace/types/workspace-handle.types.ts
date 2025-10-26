/**
 * Handle to an active workspace
 * Returned by coordinator methods
 */
export interface WorkspaceHandle {
  taskId: string
  workspacePath: string
  mode: 'local'
}
