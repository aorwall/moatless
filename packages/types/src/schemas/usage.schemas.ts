import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

export const TokenCountsSchema = z.object({
  input_tokens: z.number().min(0),
  output_tokens: z.number().min(0),
  cache_creation_tokens: z.number().min(0),
  cache_read_tokens: z.number().min(0),
  total_tokens: z.number().min(0),
})

export const UsageEntrySchema = z.object({
  timestamp: dateSchema,
  model: z.string(),
  tokenCounts: TokenCountsSchema,
  costUSD: z.number().min(0),
})

export const PerModelStatsSchema = z.object({
  tokenCounts: TokenCountsSchema,
  costUSD: z.number().min(0),
  messageCount: z.number().min(0),
})

z.object({
  startTime: dateSchema,
  endTime: dateSchema,
  entries: z.array(UsageEntrySchema),
  tokenCounts: TokenCountsSchema,
  costUSD: z.number().min(0),
  sentMessagesCount: z.number().min(0),
  perModelStats: z.record(z.string(), PerModelStatsSchema),
  isActive: z.boolean(),
})

z.object({
  cost_limit: z.number().min(0),
  token_limit: z.number().min(0),
  message_limit: z.number().min(0),
})

export const ClaudeUsageResponseSchema = z.object({
  cost: z.object({
    current: z.number().min(0),
    max: z.number().min(0),
  }),
  tokens: z.object({
    current: z.number().min(0),
    max: z.number().min(0),
  }),
  messageUsage: z.object({
    current: z.number().min(0),
    max: z.number().min(0),
  }),
  timeToReset: z.object({
    startTime: z.string(),
    endTime: z.string(),
  }),
  modelDistribution: z.record(z.string(), z.number().min(0).max(100)),
})

export const ClaudeSessionUsageResponseSchema = z.object({
  sessionId: z.string(),
  cost: z.number().min(0),
  tokens: TokenCountsSchema,
  messageCount: z.number().min(0),
  modelDistribution: z.record(z.string(), z.number().min(0).max(100)),
  startTime: z.string().optional(),
  endTime: z.string().optional(),
})

export const ClaudeUsagePlanSchema = z.enum(['pro', 'max5', 'max20', 'custom'])

export const ClaudeUsageQuerySchema = z.object({
  plan: ClaudeUsagePlanSchema.optional().default('custom'),
  repositoryId: z.string().optional(),
})

z.object({
  input: z.number().min(0),
  output: z.number().min(0),
  cache_write: z.number().min(0),
  cache_read: z.number().min(0),
})

z.object({
  cost_limit: z.number().min(0),
  token_limit: z.number().min(0),
  message_limit: z.number().min(0),
})
