import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import * as readline from 'node:readline'
import type {
  ClaudeUsagePlan,
  ClaudeUsageResponse,
  ModelPricing,
  PlanLimits,
  SessionBlock,
  TokenCounts,
  UsageEntry,
  UsageLimits,
} from '@moatless/types'
import { logger } from '../utils/logger.js'

export class ClaudeCodeUsageService {
  private readonly MODEL_PRICING: Record<string, ModelPricing> = {
    'claude-3-opus': {
      input: 15.0 / 1_000_000,
      output: 75.0 / 1_000_000,
      cache_write: 18.75 / 1_000_000,
      cache_read: 1.5 / 1_000_000,
    },
    'claude-3-5-sonnet': {
      input: 3.0 / 1_000_000,
      output: 15.0 / 1_000_000,
      cache_write: 3.75 / 1_000_000,
      cache_read: 0.3 / 1_000_000,
    },
    'claude-3-5-sonnet-20241022': {
      input: 3.0 / 1_000_000,
      output: 15.0 / 1_000_000,
      cache_write: 3.75 / 1_000_000,
      cache_read: 0.3 / 1_000_000,
    },
    'claude-3-haiku': {
      input: 0.25 / 1_000_000,
      output: 1.25 / 1_000_000,
      cache_write: 0.3 / 1_000_000,
      cache_read: 0.03 / 1_000_000,
    },
    'claude-3-5-haiku': {
      input: 1.0 / 1_000_000,
      output: 5.0 / 1_000_000,
      cache_write: 1.25 / 1_000_000,
      cache_read: 0.1 / 1_000_000,
    },
  }

  private readonly PLAN_LIMITS: Record<ClaudeUsagePlan, PlanLimits> = {
    pro: {
      cost_limit: 18.0,
      token_limit: 1_000_000,
      message_limit: 1000,
    },
    max5: {
      cost_limit: 100.0,
      token_limit: 5_000_000,
      message_limit: 5000,
    },
    max20: {
      cost_limit: 400.0,
      token_limit: 20_000_000,
      message_limit: 20000,
    },
    custom: {
      cost_limit: 0,
      token_limit: 0,
      message_limit: 0,
    },
  }

  public async getUsage(plan: ClaudeUsagePlan = 'custom'): Promise<ClaudeUsageResponse> {
    try {
      const entries = await this.readUsageEntries()
      const sortedEntries = entries.sort((a, b) => a.timestamp.getTime() - b.timestamp.getTime())
      const blocks = this.transformToBlocks(sortedEntries)
      const activeBlock = blocks.find((b) => b.isActive)

      if (!activeBlock) {
        return this.getEmptyResponse(plan)
      }

      const limits = this.getLimits(plan, blocks)
      const modelDistribution = this.calculateModelDistribution(activeBlock)

      return {
        cost: {
          current: Number(activeBlock.costUSD.toFixed(4)),
          max: limits.cost_limit,
        },
        tokens: {
          current: activeBlock.tokenCounts.total_tokens,
          max: limits.token_limit,
        },
        messageUsage: {
          current: activeBlock.sentMessagesCount,
          max: limits.message_limit,
        },
        timeToReset: {
          startTime: activeBlock.startTime.toISOString(),
          endTime: activeBlock.endTime.toISOString(),
        },
        modelDistribution,
      }
    } catch (error) {
      logger.error('Failed to get Claude usage data', { error: error as Error })
      return this.getEmptyResponse(plan)
    }
  }

  private getEmptyResponse(plan: ClaudeUsagePlan): ClaudeUsageResponse {
    const limits = this.PLAN_LIMITS[plan]
    const now = new Date()
    const endTime = new Date(now.getTime() + 5 * 60 * 60 * 1000) // 5 hours from now

    return {
      cost: { current: 0, max: limits.cost_limit },
      tokens: { current: 0, max: limits.token_limit },
      messageUsage: { current: 0, max: limits.message_limit },
      timeToReset: {
        startTime: now.toISOString(),
        endTime: endTime.toISOString(),
      },
      modelDistribution: {},
    }
  }

  private async readUsageEntries(): Promise<UsageEntry[]> {
    const entries: UsageEntry[] = []
    const homeDir = os.homedir()
    const claudePaths = [
      path.join(homeDir, '.claude', 'projects'),
      path.join(homeDir, '.config', 'claude', 'projects'),
    ]

    for (const claudePath of claudePaths) {
      try {
        const files = await this.findJsonlFiles(claudePath)
        for (const file of files) {
          const fileEntries = await this.parseJsonlFile(file)
          entries.push(...fileEntries)
        }
      } catch (error) {
        logger.debug(`Could not read Claude usage files from ${claudePath}`, {
          error: error as Error,
        })
      }
    }

    return entries
  }

  private async findJsonlFiles(dirPath: string): Promise<string[]> {
    const files: string[] = []

    try {
      const items = await fs.readdir(dirPath, { withFileTypes: true, recursive: true })

      for (const item of items) {
        if (item.isFile() && item.name.endsWith('.jsonl')) {
          const fullPath = path.join(item.parentPath || dirPath, item.name)
          files.push(fullPath)
        }
      }
    } catch (error) {
      logger.debug(`Could not read directory ${dirPath}`, { error: error as Error })
    }

    return files
  }

  private async parseJsonlFile(filePath: string): Promise<UsageEntry[]> {
    const entries: UsageEntry[] = []

    try {
      const fileStream = await fs.open(filePath, 'r')
      const rl = readline.createInterface({
        input: fileStream.createReadStream(),
        crlfDelay: Number.POSITIVE_INFINITY,
      })

      for await (const line of rl) {
        try {
          const data = JSON.parse(line)
          const entry = this.mapToUsageEntry(data)
          if (entry) {
            entries.push(entry)
          }
        } catch (parseError) {
          logger.debug(`Failed to parse line in ${filePath}`, { error: parseError as Error })
        }
      }

      await fileStream.close()
    } catch (error) {
      logger.debug(`Failed to read file ${filePath}`, { error: error as Error })
    }

    return entries
  }

  private mapToUsageEntry(data: any): UsageEntry | null {
    try {
      if (!data.timestamp) return null

      const timestamp = new Date(data.timestamp)
      if (Number.isNaN(timestamp.getTime())) return null

      const model = this.extractModel(data)
      if (!model) return null

      const tokenCounts = this.extractTokenCounts(data)
      const costUSD = this.calculateCost(model, tokenCounts)

      return {
        timestamp,
        model,
        tokenCounts,
        costUSD,
      }
    } catch (error) {
      logger.debug('Failed to map usage entry', { error: error as Error, data })
      return null
    }
  }

  private extractModel(data: any): string | null {
    return data.model || data.message?.model || null
  }

  private extractTokenCounts(data: any): TokenCounts {
    const usage = data.message?.usage || data.usage || {}

    const input_tokens = this.getTokenValue(usage, ['input_tokens', 'inputTokens']) || 0
    const output_tokens = this.getTokenValue(usage, ['output_tokens', 'outputTokens']) || 0
    const cache_creation_tokens =
      this.getTokenValue(usage, ['cache_creation_tokens', 'cacheCreationTokens']) || 0
    const cache_read_tokens =
      this.getTokenValue(usage, ['cache_read_tokens', 'cacheReadTokens']) || 0

    const total_tokens = input_tokens + output_tokens + cache_creation_tokens + cache_read_tokens

    return {
      input_tokens,
      output_tokens,
      cache_creation_tokens,
      cache_read_tokens,
      total_tokens,
    }
  }

  private getTokenValue(usage: any, keys: string[]): number | null {
    for (const key of keys) {
      if (usage[key] !== undefined && typeof usage[key] === 'number') {
        return usage[key]
      }
    }
    return null
  }

  private calculateCost(model: string, tokens: TokenCounts): number {
    const normalizedModel = this.normalizeModelName(model)
    const pricing = this.MODEL_PRICING[normalizedModel]

    if (!pricing) {
      // Ignore synthetic models from Claude Code to reduce log noise
      if (model !== '<synthetic>') {
        logger.debug(`Unknown model for pricing: ${model}`)
      }
      return 0
    }

    return (
      tokens.input_tokens * pricing.input +
      tokens.output_tokens * pricing.output +
      tokens.cache_creation_tokens * pricing.cache_write +
      tokens.cache_read_tokens * pricing.cache_read
    )
  }

  private normalizeModelName(model: string): string {
    const lowerModel = model.toLowerCase()

    if (lowerModel.includes('opus')) return 'claude-3-opus'
    if (lowerModel.includes('sonnet-20241022')) return 'claude-3-5-sonnet-20241022'
    if (lowerModel.includes('sonnet')) return 'claude-3-5-sonnet'
    if (lowerModel.includes('haiku') && lowerModel.includes('3-5')) return 'claude-3-5-haiku'
    if (lowerModel.includes('haiku')) return 'claude-3-haiku'

    return model
  }

  private transformToBlocks(entries: UsageEntry[]): SessionBlock[] {
    const blocks: SessionBlock[] = []
    let currentBlock: SessionBlock | null = null

    for (const entry of entries) {
      if (!currentBlock || this.isOutsideCurrentBlock(entry.timestamp, currentBlock)) {
        if (currentBlock) {
          blocks.push(currentBlock)
        }
        currentBlock = this.createNewBlock(entry.timestamp)
      }

      this.addEntryToBlock(entry, currentBlock)
    }

    if (currentBlock) {
      blocks.push(currentBlock)
    }

    this.markActiveBlocks(blocks)
    return blocks
  }

  private isOutsideCurrentBlock(timestamp: Date, block: SessionBlock): boolean {
    return timestamp < block.startTime || timestamp >= block.endTime
  }

  private createNewBlock(timestamp: Date): SessionBlock {
    const startTime = new Date(timestamp)
    startTime.setMinutes(0, 0, 0)

    const endTime = new Date(startTime.getTime() + 5 * 60 * 60 * 1000)

    return {
      startTime,
      endTime,
      entries: [],
      tokenCounts: {
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 0,
      },
      costUSD: 0,
      sentMessagesCount: 0,
      perModelStats: {},
      isActive: false,
    }
  }

  private addEntryToBlock(entry: UsageEntry, block: SessionBlock): void {
    block.entries.push(entry)

    block.tokenCounts.input_tokens += entry.tokenCounts.input_tokens
    block.tokenCounts.output_tokens += entry.tokenCounts.output_tokens
    block.tokenCounts.cache_creation_tokens += entry.tokenCounts.cache_creation_tokens
    block.tokenCounts.cache_read_tokens += entry.tokenCounts.cache_read_tokens
    block.tokenCounts.total_tokens += entry.tokenCounts.total_tokens

    block.costUSD += entry.costUSD
    block.sentMessagesCount += 1

    if (!block.perModelStats[entry.model]) {
      block.perModelStats[entry.model] = {
        tokenCounts: {
          input_tokens: 0,
          output_tokens: 0,
          cache_creation_tokens: 0,
          cache_read_tokens: 0,
          total_tokens: 0,
        },
        costUSD: 0,
        messageCount: 0,
      }
    }

    const modelStats = block.perModelStats[entry.model]
    modelStats.tokenCounts.input_tokens += entry.tokenCounts.input_tokens
    modelStats.tokenCounts.output_tokens += entry.tokenCounts.output_tokens
    modelStats.tokenCounts.cache_creation_tokens += entry.tokenCounts.cache_creation_tokens
    modelStats.tokenCounts.cache_read_tokens += entry.tokenCounts.cache_read_tokens
    modelStats.tokenCounts.total_tokens += entry.tokenCounts.total_tokens
    modelStats.costUSD += entry.costUSD
    modelStats.messageCount += 1
  }

  private markActiveBlocks(blocks: SessionBlock[]): void {
    const now = new Date()
    for (const block of blocks) {
      block.isActive = now >= block.startTime && now < block.endTime
    }
  }

  private getLimits(plan: ClaudeUsagePlan, blocks: SessionBlock[]): UsageLimits {
    if (plan === 'custom') {
      return this.calculateP90(blocks)
    }
    return this.PLAN_LIMITS[plan]
  }

  private calculateP90(blocks: SessionBlock[]): UsageLimits {
    const historicalBlocks = blocks.filter((block) => !block.isActive && block.entries.length > 0)

    if (historicalBlocks.length === 0) {
      return { cost_limit: 50, token_limit: 2_000_000, message_limit: 1000 }
    }

    const costs = historicalBlocks.map((block) => block.costUSD).sort((a, b) => a - b)
    const tokens = historicalBlocks
      .map((block) => block.tokenCounts.total_tokens)
      .sort((a, b) => a - b)
    const messages = historicalBlocks.map((block) => block.sentMessagesCount).sort((a, b) => a - b)

    const p90Index = Math.floor(0.9 * (costs.length - 1))

    return {
      cost_limit: costs[p90Index] || 50,
      token_limit: tokens[p90Index] || 2_000_000,
      message_limit: messages[p90Index] || 1000,
    }
  }

  private calculateModelDistribution(block: SessionBlock): Record<string, number> {
    const distribution: Record<string, number> = {}
    const totalTokens = block.tokenCounts.total_tokens

    if (totalTokens === 0) {
      return distribution
    }

    for (const [model, stats] of Object.entries(block.perModelStats)) {
      const percentage = (stats.tokenCounts.total_tokens / totalTokens) * 100
      distribution[model] = Number(percentage.toFixed(1))
    }

    return distribution
  }
}

let claudeUsageServiceInstance: ClaudeCodeUsageService | null = null

export function getClaudeUsageService(): ClaudeCodeUsageService {
  if (!claudeUsageServiceInstance) {
    claudeUsageServiceInstance = new ClaudeCodeUsageService()
  }
  return claudeUsageServiceInstance
}

export function resetClaudeUsageServiceInstance(): void {
  claudeUsageServiceInstance = null
}
