import type {
  ApiKeySource,
  Options as ClaudeCodeOptions,
  SDKMessage as ClaudeCodeSDKMessage,
  NonNullableUsage,
  PermissionMode,
  SDKAssistantMessage,
  SDKResultMessage,
  SDKSystemMessage,
  SDKUserMessage,
} from '@anthropic-ai/claude-code'

// Re-export the types we need from claude-code
export type {
  SDKAssistantMessage,
  SDKUserMessage,
  SDKResultMessage,
  SDKSystemMessage,
  NonNullableUsage,
  ApiKeySource,
  PermissionMode,
}

export type ConfigScope = 'local' | 'user' | 'project'

// Claude Code Settings
export interface ClaudeCodeSettings {
  claudeConfigPath: string
}

// Re-export from claude-code
export type { ClaudeCodeOptions }

// Export the imported type with our own name
export type { ClaudeCodeSDKMessage }
