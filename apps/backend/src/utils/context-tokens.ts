import type { ClaudeCodeSDKMessage } from '@moatless/types'

/**
 * Calculate context tokens from a Claude Code SDK message
 * Context tokens include: input_tokens + cache_read_input_tokens + cache_creation_input_tokens
 */
export function calculateContextTokens(payload: ClaudeCodeSDKMessage): number | undefined {
  if (!('message' in payload) || !('usage' in payload.message)) {
    return undefined
  }

  const usage = payload.message.usage as Record<string, unknown> | null
  if (!usage) {
    return undefined
  }

  const inputTokens = Number(usage.input_tokens || 0)
  const cacheReadTokens = Number(usage.cache_read_input_tokens || 0)
  const cacheCreationTokens = Number(usage.cache_creation_input_tokens || 0)

  return inputTokens + cacheReadTokens + cacheCreationTokens
}
