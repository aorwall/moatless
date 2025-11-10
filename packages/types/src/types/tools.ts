/**
 * Moatless Tool Types
 *
 * Type-safe definitions for Moatless MCP tools including tool names and input schemas.
 */

// ============================================================================
// Tool Names
// ============================================================================

/**
 * Union type of all available Moatless tool names (without MCP prefix)
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

// ============================================================================
// Tool Input Schemas
// ============================================================================

// ============================================================================
// Mapped Types
// ============================================================================

// ============================================================================
// Type Guards
// ============================================================================

/**
 * Strips the MCP prefix from a tool name if present
 * Handles both 'mcp__moatless-tools__toolName' and 'mcp__serverName__toolName'
 * Returns the original string if it doesn't match the pattern
 */
export function stripMcpPrefix(toolName: string): string {
  const mcpPattern = /^mcp__[^_]+__(.+)$/
  const match = toolName.match(mcpPattern)
  return match?.[1] ?? toolName
}
