import type {
  SDKMessage as ClaudeCodeSDKMessage,
  SDKAssistantMessage,
  SDKUserMessage,
  SDKResultMessage,
  SDKSystemMessage,
  NonNullableUsage,
  ApiKeySource,
  PermissionMode,
  Options as ClaudeCodeOptions,
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

// Re-export from claude-code
export type { ClaudeCodeOptions }

// Export the imported type with our own name
export { ClaudeCodeSDKMessage }
