/**
 * Moatless Tool Types
 *
 * Type-safe definitions for Moatless tools including tool names and input schemas.
 */

// ============================================================================
// Tool Names
// ============================================================================

/**
 * Union type of all available Moatless tool names
 */
export type MoatlessToolName =
  | 'updateTaskMetadata'
  | 'install'
  | 'updateRepositoryConfig'
  | 'getRepositoryConfig'
  | 'startOrRestartPreviewServer'
  | 'getPreviewServerLogs'
  | 'navigatePreview'
  | 'clickElement'
  | 'evaluateJavaScript'
  | 'fillPreviewForm'
  | 'takePreviewSnapshot'
