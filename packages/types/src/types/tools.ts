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

/**
 * Array of all Moatless tool names for runtime validation
 */
export const MOATLESS_TOOL_NAMES: readonly MoatlessToolName[] = [
  'updateTaskMetadata',
  'install',
  'updateRepositoryConfig',
  'getRepositoryConfig',
  'startOrRestartPreviewServer',
  'getPreviewServerLogs',
  'navigatePreview',
  'clickElement',
  'evaluateJavaScript',
  'fillPreviewForm',
  'takePreviewSnapshot',
] as const

// ============================================================================
// Tool Input Schemas
// ============================================================================

/**
 * Input schema for updateTaskMetadata tool
 */
export interface UpdateTaskMetadataInput {
  name?: string
  description?: string
}

/**
 * Input schema for install tool
 */
export interface InstallInput {
  command: string
  workingDirectory?: string
  timeout?: number
}

/**
 * Input schema for updateRepositoryConfig tool
 */
export interface UpdateRepositoryConfigInput {
  config: {
    owner?: string
    name?: string
    branch?: string
    setupCommands?: string[]
    [key: string]: unknown
  }
}

/**
 * Input schema for getRepositoryConfig tool (no parameters)
 */
export type GetRepositoryConfigInput = Record<string, never>

/**
 * Input schema for startOrRestartPreviewServer tool
 */
export interface StartOrRestartPreviewServerInput {
  command?: string
  port?: number
  workingDirectory?: string
  env?: Record<string, string>
}

/**
 * Input schema for getPreviewServerLogs tool
 */
export interface GetPreviewServerLogsInput {
  lines?: number
  filterPattern?: string
}

/**
 * Input schema for navigatePreview tool
 */
export interface NavigatePreviewInput {
  url: string
  waitUntil?: 'load' | 'domcontentloaded' | 'networkidle'
  timeout?: number
}

/**
 * Input schema for clickElement tool
 */
export interface ClickElementInput {
  selector: string
  timeout?: number
  waitForNavigation?: boolean
}

/**
 * Input schema for evaluateJavaScript tool
 */
export interface EvaluateJavaScriptInput {
  script: string
  args?: unknown[]
}

/**
 * Input schema for fillPreviewForm tool
 */
export interface FillPreviewFormInput {
  fields: Array<{
    selector: string
    value: string
  }>
  submit?: boolean
  submitSelector?: string
}

/**
 * Input schema for takePreviewSnapshot tool
 */
export interface TakePreviewSnapshotInput {
  fullPage?: boolean
  selector?: string
  quality?: number
}

// ============================================================================
// Mapped Types
// ============================================================================

/**
 * Mapped type that associates each tool name with its input type
 */
export type MoatlessToolInputMap = {
  updateTaskMetadata: UpdateTaskMetadataInput
  install: InstallInput
  updateRepositoryConfig: UpdateRepositoryConfigInput
  getRepositoryConfig: GetRepositoryConfigInput
  startOrRestartPreviewServer: StartOrRestartPreviewServerInput
  getPreviewServerLogs: GetPreviewServerLogsInput
  navigatePreview: NavigatePreviewInput
  clickElement: ClickElementInput
  evaluateJavaScript: EvaluateJavaScriptInput
  fillPreviewForm: FillPreviewFormInput
  takePreviewSnapshot: TakePreviewSnapshotInput
}

/**
 * Get the input type for a specific tool name
 */
export type MoatlessToolInput<T extends MoatlessToolName> = MoatlessToolInputMap[T]

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
