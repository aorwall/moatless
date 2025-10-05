export interface TokenCounts {
  input_tokens: number
  output_tokens: number
  cache_creation_tokens: number
  cache_read_tokens: number
  total_tokens: number
}

export interface PerModelStats {
  tokenCounts: TokenCounts
  costUSD: number
  messageCount: number
}

// Placeholder type to avoid circular dependency with SDKMessage
type SDKMessagePlaceholder = Record<string, unknown>

export interface SessionBlock {
  startTime: Date
  endTime: Date
  entries: SDKMessagePlaceholder[] // SDKMessage[] - using placeholder to avoid circular dependency
  tokenCounts: TokenCounts
  costUSD: number
  sentMessagesCount: number
  perModelStats: Record<string, PerModelStats>
  isActive: boolean
}

export interface UsageLimits {
  cost_limit: number
  token_limit: number
  message_limit: number
}

export interface ClaudeUsageResponse {
  cost: {
    current: number
    max: number
  }
  tokens: {
    current: number
    max: number
  }
  messageUsage: {
    current: number
    max: number
  }
  timeToReset: {
    startTime: string
    endTime: string
  }
  modelDistribution: Record<string, number>
}

export interface ClaudeSessionUsageResponse {
  sessionId: string
  cost: number
  tokens: TokenCounts
  messageCount: number
  modelDistribution: Record<string, number>
  startTime: string | undefined
  endTime: string | undefined
}

export type ClaudeUsagePlan = 'pro' | 'max5' | 'max20' | 'custom'

export interface ModelPricing {
  input: number
  output: number
  cache_write: number
  cache_read: number
}

export interface PlanLimits {
  cost_limit: number
  token_limit: number
  message_limit: number
}
