import type { AgentState, PreviewState } from './workspace-runtime.types'

/**
 * Preview state with derived helper
 */
export interface WorkspaceHandlePreviewState extends Omit<PreviewState, 'endpoint' | 'proxyUrl'> {
  /** True if setup commands have never been run (no lastStartedAt) */
  needsSetup: boolean
}

/**
 * Base workspace handle without runtime state
 * Returned by WorkspaceMaterializer (provisioning layer)
 */
export interface BaseWorkspaceHandle {
  taskId: string
  workspacePath: string
  mode: 'local' | 'worker'
}

/**
 * Handle to an active workspace with runtime state
 * Returned by RuntimeManager methods
 *
 * This enriched handle eliminates the need for consumers to
 * load runtime snapshots directly - all necessary state is included.
 */
export interface WorkspaceHandle extends BaseWorkspaceHandle {
  repositoryId: string

  /** Preview server runtime state */
  preview: WorkspaceHandlePreviewState

  /** Agent execution runtime state */
  agent: AgentState
}
