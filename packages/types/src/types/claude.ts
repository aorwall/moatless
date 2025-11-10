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
} from '@anthropic-ai/claude-agent-sdk'

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

// Claude Code Settings

// Re-export from claude-code
export type { ClaudeCodeOptions }

// Export the imported type with our own name
export type { ClaudeCodeSDKMessage }
