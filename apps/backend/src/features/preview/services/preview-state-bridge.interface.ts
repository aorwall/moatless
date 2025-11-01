import type { PreviewState } from '../../workspace/types/workspace-runtime.types.js'

/**
 * PreviewStateBridge - Interface for workspace to update preview state
 *
 * This interface breaks the circular dependency between preview and workspace features.
 * The preview feature implements this interface and workspace feature depends on it.
 *
 * Design principle: Minimal interface with only what workspace needs to update preview state.
 */
export interface PreviewStateBridge {
  /**
   * Update preview state in workspace snapshot
   */
  updatePreviewState(taskId: string, updates: Partial<PreviewState>): Promise<void>
}
