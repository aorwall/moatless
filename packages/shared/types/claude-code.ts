import type { PermissionMode } from './claude.js'

export type { PermissionMode } from './claude.js'

export interface ClaudeCodeTool {
  name: string
  description: string
  permissionRequired: boolean
}

export interface ClaudeCodeToolsResponse {
  tools: ClaudeCodeTool[]
}

export interface ClaudeCodeSettingsResponse {
  model: string | null
  maxTurns: number | null
  maxThinkingTokens: number
  systemPrompt: string | null
  appendSystemPrompt: string | null
  allowedTools: string[]
  disallowedTools: string[]
  permissionMode: PermissionMode | null
}

export interface UpdateClaudeCodeSettingsRequest {
  model: string | null
  maxTurns: number | null
  maxThinkingTokens: number
  systemPrompt: string | null
  appendSystemPrompt: string | null
  allowedTools: string[]
  disallowedTools: string[]
  permissionMode: PermissionMode | null
}

export interface BranchNameAndSummary {
  branchName: string
  summary?: string
}
