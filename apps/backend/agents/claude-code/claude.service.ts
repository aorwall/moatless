import { existsSync, promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type {
  SDKAssistantMessage as SDKAssistantMessageFromClaude,
  SDKMessage,
} from '@anthropic-ai/claude-code'
import type {
  AgentMessage,
  ClaudeSessionUsageResponse,
  ClaudeUsagePlan,
  ClaudeUsageResponse,
  ModelPricing,
  PerModelStats,
  RepositoryConfig,
  SessionBlock,
  Task,
  TokenCounts,
  UsageLimits,
} from '@moatless/types'
import type { SettingsService } from '../../services/settings.service.js'
import { logger } from '../../utils/logger.js'
import {
  findJsonlFiles,
  getClaudeDataPaths,
  parseJsonlFile,
  readFileLines,
} from './claude-utils.js'

export class ClaudeService {
  private messageFilePathCache: Map<string, string> = new Map()

  constructor(private settingsService: SettingsService) {}

  // Model pricing per million tokens (as of Nov 2024)
  private readonly modelPricing: Record<string, ModelPricing> = {
    'claude-3-5-sonnet-20241022': {
      input: 3.0,
      output: 15.0,
      cache_write: 3.75,
      cache_read: 0.3,
    },
    'claude-3-5-sonnet-20240620': {
      input: 3.0,
      output: 15.0,
      cache_write: 3.75,
      cache_read: 0.3,
    },
    'claude-3-5-haiku-20241022': {
      input: 1.0,
      output: 5.0,
      cache_write: 1.25,
      cache_read: 0.1,
    },
    'claude-3-opus-20240229': {
      input: 15.0,
      output: 75.0,
      cache_write: 18.75,
      cache_read: 1.5,
    },
    'claude-3-haiku-20240307': {
      input: 0.25,
      output: 1.25,
      cache_write: 0.3,
      cache_read: 0.03,
    },
  }

  // Plan limits with corrected token values (matching Python implementation)
  private readonly planLimits: Record<ClaudeUsagePlan, UsageLimits> = {
    pro: {
      cost_limit: 18,
      token_limit: 19_000,
      message_limit: 1000,
    },
    max5: {
      cost_limit: 35,
      token_limit: 88_000,
      message_limit: 5000,
    },
    max20: {
      cost_limit: 140,
      token_limit: 220_000,
      message_limit: 20000,
    },
    custom: {
      cost_limit: 0,
      token_limit: 0,
      message_limit: 0,
    },
  }

  /**
   * Get Claude usage data with plan limits
   */
  public async getUsage(
    plan?: ClaudeUsagePlan,
    repository?: RepositoryConfig,
  ): Promise<ClaudeUsageResponse> {
    const actualPlan = plan || 'custom'
    const planLimit = this.planLimits[actualPlan]

    // Get current usage
    const currentBlock = await this.getCurrentSessionBlock(repository)

    // For custom plan, calculate P90 limit or use default
    let customLimit: UsageLimits = { cost_limit: 0, token_limit: 44_000, message_limit: 0 }
    if (actualPlan === 'custom') {
      const historicalLimit = await this.calculateHistoricalP90Limit(repository)
      if (historicalLimit) {
        customLimit = historicalLimit
      }
    }

    const maxCost = actualPlan === 'custom' ? customLimit.cost_limit : planLimit.cost_limit
    const maxTokens = actualPlan === 'custom' ? customLimit.token_limit : planLimit.token_limit
    const maxMessages =
      actualPlan === 'custom' ? customLimit.message_limit : planLimit.message_limit

    // Calculate model distribution percentages
    const modelDistribution: Record<string, number> = {}
    if (currentBlock?.perModelStats) {
      const totalTokens = currentBlock.tokenCounts.total_tokens
      for (const [model, stats] of Object.entries(currentBlock.perModelStats)) {
        if (totalTokens > 0) {
          const percentage = Math.round((stats.tokenCounts.total_tokens / totalTokens) * 100)
          modelDistribution[model] = percentage
        }
      }
    }

    return {
      cost: {
        current: currentBlock?.costUSD || 0,
        max: maxCost,
      },
      tokens: {
        current: currentBlock?.tokenCounts.total_tokens || 0,
        max: maxTokens,
      },
      messageUsage: {
        current: currentBlock?.sentMessagesCount || 0,
        max: maxMessages,
      },
      timeToReset: {
        startTime: currentBlock?.startTime.toISOString() || new Date().toISOString(),
        endTime: currentBlock?.endTime.toISOString() || new Date().toISOString(),
      },
      modelDistribution,
    }
  }

  /**
   * Get usage data for a specific session
   */
  public async getSessionUsage(sessionId: string): Promise<ClaudeSessionUsageResponse> {
    const entries = await this.readSessionEntries(sessionId)

    if (entries.length === 0) {
      return {
        sessionId,
        cost: 0,
        tokens: {
          input_tokens: 0,
          output_tokens: 0,
          cache_creation_tokens: 0,
          cache_read_tokens: 0,
          total_tokens: 0,
        },
        messageCount: 0,
        modelDistribution: {},
      }
    }

    // Calculate aggregated stats
    const tokenCounts: TokenCounts = {
      input_tokens: 0,
      output_tokens: 0,
      cache_creation_tokens: 0,
      cache_read_tokens: 0,
      total_tokens: 0,
    }

    let totalCost = 0
    const perModelStats: Record<string, PerModelStats> = {}
    let startTime: Date | undefined
    let endTime: Date | undefined

    for (const entry of entries) {
      if (entry.message?.usage) {
        // Update token counts
        tokenCounts.input_tokens += entry.message.usage.input_tokens || 0
        tokenCounts.output_tokens += entry.message.usage.output_tokens || 0
        tokenCounts.cache_creation_tokens += entry.message.usage.cache_creation_input_tokens || 0
        tokenCounts.cache_read_tokens += entry.message.usage.cache_read_input_tokens || 0

        // Calculate cost for this message
        const model = entry.message.model || 'unknown'
        const pricing = this.modelPricing[model]
        if (pricing) {
          const cost = this.calculateMessageCost(entry.message.usage, pricing)
          totalCost += cost

          // Update per-model stats
          if (!perModelStats[model]) {
            perModelStats[model] = {
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

          perModelStats[model].tokenCounts.input_tokens += entry.message.usage.input_tokens || 0
          perModelStats[model].tokenCounts.output_tokens += entry.message.usage.output_tokens || 0
          perModelStats[model].tokenCounts.cache_creation_tokens +=
            entry.message.usage.cache_creation_input_tokens || 0
          perModelStats[model].tokenCounts.cache_read_tokens +=
            entry.message.usage.cache_read_input_tokens || 0
          perModelStats[model].costUSD += cost
          perModelStats[model].messageCount++
        }

        // Track time range
        if ('timestamp' in entry && entry.timestamp) {
          const timestamp = new Date(entry.timestamp as string)
          if (!startTime || timestamp < startTime) startTime = timestamp
          if (!endTime || timestamp > endTime) endTime = timestamp
        }
      }
    }

    // Update total tokens
    tokenCounts.total_tokens = tokenCounts.input_tokens + tokenCounts.output_tokens

    // Calculate model distribution percentages
    const modelDistribution: Record<string, number> = {}
    if (tokenCounts.total_tokens > 0) {
      for (const [model, stats] of Object.entries(perModelStats)) {
        const totalModelTokens = stats.tokenCounts.input_tokens + stats.tokenCounts.output_tokens
        const percentage = Math.round((totalModelTokens / tokenCounts.total_tokens) * 100)
        modelDistribution[model] = percentage
      }
    }

    return {
      sessionId,
      cost: totalCost,
      tokens: tokenCounts,
      messageCount: entries.length,
      modelDistribution,
      startTime: startTime?.toISOString(),
      endTime: endTime?.toISOString(),
    }
  }

  // ===============================
  // Private Helper Methods
  // ===============================

  /**
   * Get Claude projects directory path
   */
  public getClaudeProjectPath(): string {
    const claudeSettings = this.settingsService.getClaudeSettings()

    // Use configDir from settings if available, otherwise use default
    const claudeConfigDir = claudeSettings.configDir || path.join(os.homedir(), '.claude')
    return path.join(claudeConfigDir, 'projects')
  }

  /**
   * Read Claude config.json file and return as object
   */
  public async readClaudeConfig(): Promise<Record<string, any>> {
    const claudeSettings = this.settingsService.getClaudeSettings()
    const claudeConfigDir = claudeSettings.configDir || path.join(os.homedir(), '.claude')
    const configPath = path.join(claudeConfigDir, '.config.json')

    try {
      const configContent = await fs.readFile(configPath, 'utf-8')
      return JSON.parse(configContent)
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        throw new Error(`Claude config.json file does not exist on path ${configPath}`)
      }
      logger.error('Error reading Claude config.json', error as Error, { configPath })
      throw error
    }
  }

  /**
   * Get all project directories from Claude config
   */
  public async getClaudeProjectDirectories(): Promise<string[]> {
    const config = await this.readClaudeConfig()

    if (!config.projects || typeof config.projects !== 'object') {
      logger.warn('Claude config.json does not contain projects object', { config })
      return []
    }

    // Extract all keys from the projects object, which are the file paths
    return Object.keys(config.projects)
  }

  /**
   * Generate project ID from path, using repository path for docker environments
   */
  public generateProjectId(task: Task, repository: RepositoryConfig): string {
    // For docker environments, use repository path instead of workspace path
    logger.info('Generating project ID', { repository })
    if (repository.codeExecution.type === 'docker') {
      return repository.path.replace(/[^a-zA-Z0-9]/g, '-')
    }

    // For local environments, use workspace path
    const workspacePath = task.workspacePath
    if (!workspacePath) {
      throw new Error('No workspace path available for generating project ID')
    }

    return workspacePath.replace(/[^a-zA-Z0-9]/g, '-')
  }

  /**
   * Find messages file by searching for {agentSessionId}.jsonl in any subdirectory
   */
  private async findMessagesFile(agentSessionId: string): Promise<string | null> {
    // Check cache first
    const cached = this.messageFilePathCache.get(agentSessionId)
    if (cached) {
      // Verify the file still exists
      try {
        await fs.access(cached, fs.constants.F_OK)
        return cached
      } catch {
        // File no longer exists, remove from cache
        this.messageFilePathCache.delete(agentSessionId)
      }
    }

    // Search for the file
    const claudeProjectPath = this.getClaudeProjectPath()
    const targetFileName = `${agentSessionId}.jsonl`

    try {
      const allFiles = await findJsonlFiles(claudeProjectPath)

      for (const filePath of allFiles) {
        if (path.basename(filePath) === targetFileName) {
          // Cache the result
          this.messageFilePathCache.set(agentSessionId, filePath)
          return filePath
        }
      }
    } catch (error) {
      logger.error('Error searching for messages file', error as Error, {
        agentSessionId,
        claudeProjectPath,
      })
    }

    return null
  }

  /**
   * Get Claude messages file path for a specific agent session
   */
  public async getClaudeMessagesFilePath(
    agentSessionId: string,
    projectId?: string,
  ): Promise<string> {
    // Try to find the file first
    const foundPath = await this.findMessagesFile(agentSessionId)
    if (foundPath) {
      return foundPath
    }

    // If not found and projectId is provided, return the expected path
    if (projectId) {
      const claudeProjectPath = this.getClaudeProjectPath()
      return path.join(claudeProjectPath, projectId, `${agentSessionId}.jsonl`)
    }

    // If no projectId and file not found, return a default path
    const claudeProjectPath = this.getClaudeProjectPath()
    return path.join(claudeProjectPath, `${agentSessionId}.jsonl`)
  }

  /**
   * Read Claude messages for a specific agent session
   */
  public async readClaudeMessages(agentSessionId: string): Promise<AgentMessage[]> {
    const messages: AgentMessage[] = []

    // Try to find the session file
    const filePath = await this.findMessagesFile(agentSessionId)

    if (!filePath) {
      logger.debug('No Claude messages file found for session', { agentSessionId })
      return messages
    }

    try {
      const fileContent = await fs.readFile(filePath, 'utf-8')
      const lines = fileContent.trim().split('\n').filter(Boolean)

      for (const line of lines) {
        try {
          const agentMessage = JSON.parse(line) as AgentMessage

          // Transform ExitPlanMode tool calls to regular text messages
          const transformedMessage = this.transformExitPlanModeMessage(agentMessage)
          messages.push(transformedMessage)
        } catch (parseError) {
          logger.warn('Failed to parse Claude agent message', {
            agentSessionId,
            error: parseError instanceof Error ? parseError.message : String(parseError),
          })
        }
      }

      logger.debug('Read Claude messages from file', {
        agentSessionId,
        filePath,
        messageCount: messages.length,
      })
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        logger.debug('Claude messages file does not exist', {
          agentSessionId,
          filePath,
        })
      } else {
        logger.error('Error reading Claude messages file', error as Error, {
          agentSessionId,
          filePath,
        })
      }
    }

    return messages
  }

  /**
   * Transform ExitPlanMode tool calls into regular text messages
   */
  private transformExitPlanModeMessage(message: AgentMessage): AgentMessage {
    // Only process assistant messages
    if (message.type !== 'assistant') {
      return message
    }

    // Check if message has content
    if (
      !('message' in message) ||
      !message.message ||
      !('content' in message.message) ||
      !message.message.content
    ) {
      return message
    }

    const content = Array.isArray(message.message.content)
      ? message.message.content
      : [message.message.content]
    let hasExitPlanMode = false
    let extractedPlan: string | null = null

    // Look for ExitPlanMode tool calls
    for (const contentItem of content) {
      if (
        contentItem &&
        typeof contentItem === 'object' &&
        'type' in contentItem &&
        contentItem.type === 'tool_use' &&
        'name' in contentItem &&
        contentItem.name === 'ExitPlanMode' &&
        'input' in contentItem &&
        contentItem.input &&
        typeof contentItem.input === 'object' &&
        'plan' in contentItem.input
      ) {
        hasExitPlanMode = true
        extractedPlan = contentItem.input.plan as string
        logger.debug('Transforming ExitPlanMode tool call to text message', {
          planLength: extractedPlan?.length,
        })
        break
      }
    }

    // Transform the message if ExitPlanMode was found
    if (hasExitPlanMode && extractedPlan) {
      return {
        ...message,
        message: {
          ...message.message,
          content: [
            {
              type: 'text',
              text: extractedPlan,
            },
          ],
        },
      }
    }

    return message
  }

  private calculateMessageCost(usage: any, pricing: ModelPricing): number {
    const inputCost = ((usage.input_tokens || 0) / 1_000_000) * pricing.input
    const outputCost = ((usage.output_tokens || 0) / 1_000_000) * pricing.output
    const cacheWriteCost =
      ((usage.cache_creation_input_tokens || 0) / 1_000_000) * pricing.cache_write
    const cacheReadCost = ((usage.cache_read_input_tokens || 0) / 1_000_000) * pricing.cache_read

    return inputCost + outputCost + cacheWriteCost + cacheReadCost
  }

  private async getCurrentSessionBlock(
    repository?: RepositoryConfig,
  ): Promise<SessionBlock | null> {
    const now = Date.now()
    const fiveHoursMs = 5 * 60 * 60 * 1000

    // Calculate current 5-hour window
    const windowStart = Math.floor(now / fiveHoursMs) * fiveHoursMs
    const windowEnd = windowStart + fiveHoursMs

    const entries = await this.readUsageEntries(5, repository) // Read last 5 hours

    if (entries.length === 0) {
      return null
    }

    return this.createSessionBlock(entries, new Date(windowStart), new Date(windowEnd))
  }

  private async calculateHistoricalP90Limit(
    repository?: RepositoryConfig,
  ): Promise<UsageLimits | null> {
    // Read 96 hours (4 days) of historical data
    const entries = await this.readUsageEntries(96, repository)

    if (entries.length === 0) {
      return null
    }

    // Group entries into 5-hour windows
    const blocks: SessionBlock[] = []
    const fiveHoursMs = 5 * 60 * 60 * 1000

    // Find the time range
    const timestamps: number[] = []
    for (const entry of entries) {
      if (entry.type === 'assistant' && 'timestamp' in entry && entry.timestamp) {
        timestamps.push(new Date(entry.timestamp as string).getTime())
      }
    }

    if (timestamps.length === 0) {
      return null
    }

    const minTime = Math.min(...timestamps)
    const maxTime = Math.max(...timestamps)

    // Create blocks for each 5-hour window
    for (let windowStart = minTime; windowStart <= maxTime; windowStart += fiveHoursMs) {
      const windowEnd = windowStart + fiveHoursMs
      const windowEntries: SDKMessage[] = []
      for (const entry of entries) {
        if (entry.type === 'assistant' && 'timestamp' in entry && entry.timestamp) {
          const timestamp = new Date(entry.timestamp as string).getTime()
          if (timestamp >= windowStart && timestamp < windowEnd) {
            windowEntries.push(entry)
          }
        }
      }

      if (windowEntries.length > 0) {
        const block = this.createSessionBlock(
          windowEntries,
          new Date(windowStart),
          new Date(windowEnd),
        )
        blocks.push(block)
      }
    }

    if (blocks.length === 0) {
      return null
    }

    // Calculate P90 values
    const costs = blocks.map((b) => b.costUSD).sort((a, b) => a - b)
    const tokens = blocks.map((b) => b.tokenCounts.total_tokens).sort((a, b) => a - b)
    const messages = blocks.map((b) => b.sentMessagesCount).sort((a, b) => a - b)

    const p90Index = Math.floor(blocks.length * 0.9)

    return {
      cost_limit: costs[p90Index] || 0,
      token_limit: tokens[p90Index] || 0,
      message_limit: messages[p90Index] || 0,
    }
  }

  private async readSessionEntries(sessionId: string): Promise<SDKAssistantMessageFromClaude[]> {
    const entries: SDKAssistantMessageFromClaude[] = []

    // Try to find the session file using our cache-enabled method
    const sessionFilePath = await this.findMessagesFile(sessionId)

    if (sessionFilePath) {
      try {
        const lines = await readFileLines(sessionFilePath)
        for (const line of lines) {
          try {
            const message = JSON.parse(line) as SDKMessage
            if (message.type === 'assistant') {
              entries.push(message as SDKAssistantMessageFromClaude)
            }
          } catch (parseError) {
            logger.warn('Failed to parse session entry', {
              sessionId,
              error: parseError instanceof Error ? parseError.message : String(parseError),
            })
          }
        }
      } catch (error) {
        logger.error('Error reading session file', error as Error, {
          sessionId,
          sessionFilePath,
        })
      }
    } else {
      logger.debug('Session file not found', { sessionId })
    }

    return entries
  }

  private createSessionBlock(entries: SDKMessage[], startTime: Date, endTime: Date): SessionBlock {
    const tokenCounts: TokenCounts = {
      input_tokens: 0,
      output_tokens: 0,
      cache_creation_tokens: 0,
      cache_read_tokens: 0,
      total_tokens: 0,
    }

    let costUSD = 0
    let sentMessagesCount = 0
    const perModelStats: Record<string, PerModelStats> = {}

    for (const entry of entries) {
      if (entry.type === 'assistant' && entry.message?.usage) {
        sentMessagesCount++

        // Update token counts
        tokenCounts.input_tokens += entry.message.usage.input_tokens || 0
        tokenCounts.output_tokens += entry.message.usage.output_tokens || 0
        tokenCounts.cache_creation_tokens += entry.message.usage.cache_creation_input_tokens || 0
        tokenCounts.cache_read_tokens += entry.message.usage.cache_read_input_tokens || 0

        // Calculate cost
        const model = entry.message.model || 'unknown'
        const pricing = this.modelPricing[model]
        if (pricing) {
          const messageCost = this.calculateMessageCost(entry.message.usage, pricing)
          costUSD += messageCost

          // Update per-model stats
          if (!perModelStats[model]) {
            perModelStats[model] = {
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

          perModelStats[model].tokenCounts.input_tokens += entry.message.usage.input_tokens || 0
          perModelStats[model].tokenCounts.output_tokens += entry.message.usage.output_tokens || 0
          perModelStats[model].tokenCounts.cache_creation_tokens +=
            entry.message.usage.cache_creation_input_tokens || 0
          perModelStats[model].tokenCounts.cache_read_tokens +=
            entry.message.usage.cache_read_input_tokens || 0
          perModelStats[model].costUSD += messageCost
          perModelStats[model].messageCount++
        }
      }
    }

    // Update total tokens
    tokenCounts.total_tokens = tokenCounts.input_tokens + tokenCounts.output_tokens

    // Update per-model total tokens
    for (const stats of Object.values(perModelStats)) {
      stats.tokenCounts.total_tokens =
        stats.tokenCounts.input_tokens + stats.tokenCounts.output_tokens
    }

    return {
      startTime,
      endTime,
      entries,
      tokenCounts,
      costUSD,
      sentMessagesCount,
      perModelStats,
      isActive: endTime > new Date(),
    }
  }

  private async readUsageEntries(
    hoursBack?: number,
    repository?: RepositoryConfig,
  ): Promise<SDKMessage[]> {
    const entries: SDKMessage[] = []

    // For usage entries, we need to search in all possible project directories
    const claudePaths =
      repository && repository.codeExecution.type === 'docker'
        ? [this.getClaudeProjectPath()] // Search in all project directories
        : getClaudeDataPaths() // Use standard paths for local

    // Default to 96 hours (4 days) like Python implementation if not specified
    const maxAgeMs = hoursBack !== undefined ? hoursBack * 60 * 60 * 1000 : undefined

    // Track seen UUIDs and session IDs across all files
    const seenUuids = new Set<string>()
    const processedSessionIds = new Set<string>()

    for (const claudePath of claudePaths) {
      try {
        // Find all JSONL files, filtered by age if specified
        const allFiles = await findJsonlFiles(claudePath, maxAgeMs)

        for (const file of allFiles) {
          const fileEntries = await parseJsonlFile<SDKMessage>(file, seenUuids, processedSessionIds)

          // Filter to only assistant messages (they have usage data)
          const assistantEntries = fileEntries.filter(
            (e): e is SDKAssistantMessageFromClaude => e.type === 'assistant',
          )
          entries.push(...assistantEntries)

          // Track processed session IDs
          const sessionId = path.basename(file, '.jsonl')
          processedSessionIds.add(sessionId)
        }
      } catch (error) {
        logger.debug('Error reading usage entries from directory', {
          claudePath,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    // Sort by timestamp (newest first)
    entries.sort((a, b) => {
      let aTime = 0
      let bTime = 0
      if ('timestamp' in a && a.timestamp) {
        aTime = new Date(a.timestamp as string).getTime()
      }
      if ('timestamp' in b && b.timestamp) {
        bTime = new Date(b.timestamp as string).getTime()
      }
      return bTime - aTime
    })

    return entries
  }

  // ===============================
  // Docker-specific Configuration Methods
  // ===============================

  /**
   * Get Claude configuration file paths for Docker environment setup
   */
  getClaudeConfigPaths(): {
    configPath: string | null
    credentialsPath: string | null
    claudeConfigDir: string
  } {
    const claudeSettings = this.settingsService.getClaudeSettings()

    // Determine Claude config directory
    let claudeConfigDir: string
    if (claudeSettings.configDir) {
      claudeConfigDir = claudeSettings.configDir
    } else if (process.env.CLAUDE_CONFIG_HOME) {
      claudeConfigDir = process.env.CLAUDE_CONFIG_HOME
    } else {
      claudeConfigDir = path.join(process.env.HOME || '', '.claude')
    }

    let claudeConfigPath: string | null = null
    let foundCredentialsPath: string | null = null

    // First try CLAUDE_CONFIG_HOME/.config.json, then fallback to USER_HOME/.claude.json
    const configJsonPath = path.join(claudeConfigDir, '.config.json')
    if (existsSync(configJsonPath)) {
      claudeConfigPath = configJsonPath
    }

    if (!claudeConfigPath) {
      const claudeJsonPath = path.join(process.env.HOME || '', '.claude.json')
      if (existsSync(claudeJsonPath)) {
        claudeConfigPath = claudeJsonPath
      }
    }

    // Check for credentials file - try both .credentials.json and .credentials
    const credentialsJsonPath = path.join(claudeConfigDir, '.credentials.json')
    const credentialsPath = path.join(claudeConfigDir, '.credentials')
    if (existsSync(credentialsJsonPath)) {
      foundCredentialsPath = credentialsJsonPath
    } else if (existsSync(credentialsPath)) {
      foundCredentialsPath = credentialsPath
    }

    return {
      configPath: claudeConfigPath,
      credentialsPath: foundCredentialsPath,
      claudeConfigDir,
    }
  }

  /**
   * Validate Claude authentication for Docker environment
   */
  validateClaudeAuthentication(): {
    isValid: boolean
    hasConfig: boolean
    hasCredentials: boolean
    hasOAuthToken: boolean
    error?: string
  } {
    const { configPath, credentialsPath } = this.getClaudeConfigPaths()
    const claudeOauthToken = this.settingsService.getClaudeSettings().oauthToken

    const hasConfig = !!configPath
    const hasCredentials = !!credentialsPath
    const hasOAuthToken = !!claudeOauthToken

    // Fail if no credentials found and no OAuth token when Claude config exists
    if (!hasCredentials && !hasOAuthToken && hasConfig) {
      return {
        isValid: false,
        hasConfig,
        hasCredentials,
        hasOAuthToken,
        error:
          'No Claude credentials found (.credentials or .credentials.json) and no OAuth token configured - Claude Code agent will not work. Run `claude setup-token` to get an OAuth token.',
      }
    }

    if (!hasCredentials && !hasOAuthToken) {
      return {
        isValid: false,
        hasConfig,
        hasCredentials,
        hasOAuthToken,
        error:
          'No Claude credentials or OAuth token found - Claude Code agent may not work properly',
      }
    }

    return {
      isValid: true,
      hasConfig,
      hasCredentials,
      hasOAuthToken,
    }
  }

  /**
   * Find Claude bin directory by checking common locations
   */
  findClaudeBinDir(): string {
    // Check common locations for Claude executable
    // Prioritize system and npm global locations
    const possiblePaths = [
      '/usr/bin', // Common system location (as found on your server)
      '/usr/local/bin',
    ]

    // Add npm global bin directories (for npm-installed claude)
    // Check for nvm paths
    if (process.env.NVM_BIN) {
      possiblePaths.push(process.env.NVM_BIN)
    }

    // Check for standard npm global paths
    const npmPrefixes = [
      path.join(os.homedir(), '.npm-global'),
      path.join(os.homedir(), '.nvm', 'versions', 'node'), // nvm installations
      '/usr/local', // npm global prefix on many systems
    ]

    for (const prefix of npmPrefixes) {
      const npmBinPath = path.join(prefix, 'bin')
      if (existsSync(npmBinPath)) {
        possiblePaths.push(npmBinPath)
      }
      // Also check nvm version subdirectories
      if (prefix.includes('.nvm')) {
        try {
          const versionDirs = require('fs').readdirSync(path.join(prefix))
          for (const version of versionDirs) {
            const versionBinPath = path.join(prefix, version, 'bin')
            if (existsSync(versionBinPath)) {
              possiblePaths.push(versionBinPath)
            }
          }
        } catch {
          // Ignore errors reading nvm directories
        }
      }
    }

    // Add user-specific paths
    possiblePaths.push(path.join(os.homedir(), '.local', 'bin'), path.join(os.homedir(), 'bin'))

    // Add PATH directories as fallback
    if (process.env.PATH) {
      const pathDirs = process.env.PATH.split(path.delimiter)
      possiblePaths.push(...pathDirs)
    }

    // Look for claude executable in each path
    for (const dir of possiblePaths) {
      const claudePath = path.join(dir, 'claude')
      if (existsSync(claudePath)) {
        return dir
      }
    }

    return ''
  }

  /**
   * Find Claude config directory
   */
  findClaudeConfigDir(): string {
    // First check CLAUDE_CONFIG_DIR env var
    if (process.env.CLAUDE_CONFIG_DIR && existsSync(process.env.CLAUDE_CONFIG_DIR)) {
      return process.env.CLAUDE_CONFIG_DIR
    }

    // Check default location
    const defaultConfigDir = path.join(os.homedir(), '.claude')
    if (existsSync(defaultConfigDir)) {
      return defaultConfigDir
    }

    // Return default even if it doesn't exist yet
    return defaultConfigDir
  }

  /**
   * Validate Claude paths and return their validity status
   */
  async validateClaudePaths(
    binDir: string,
    configDir: string,
  ): Promise<{
    binDirValid: boolean
    configDirValid: boolean
    claudeVersion?: string
    error?: string
  }> {
    // Check if binDir exists and contains claude executable
    let binDirValid = false
    let claudeVersion: string | undefined
    let error: string | undefined

    if (binDir) {
      const claudeBinPath = path.join(binDir, 'claude')

      // First check if file exists
      if (existsSync(claudeBinPath)) {
        // Try to execute claude --version to verify it's working
        try {
          const { exec } = await import('node:child_process')
          const { promisify } = await import('node:util')
          const execAsync = promisify(exec)

          const { stdout, stderr } = await execAsync(`"${claudeBinPath}" --version`, {
            timeout: 5000, // 5 second timeout
            env: {
              ...process.env,
              HOME: process.env.HOME || '/home/azureuser',
              USER: process.env.USER || 'azureuser',
            },
          })

          if (stdout) {
            binDirValid = true
            claudeVersion = stdout.trim()
            logger.info('Claude binary validated successfully', {
              claudeBinPath,
              version: claudeVersion,
            })
          } else if (stderr) {
            error = `Claude binary error: ${stderr}`
            logger.error('Claude binary failed validation', new Error(stderr), {
              claudeBinPath,
            })
          }
        } catch (execError) {
          error = `Failed to execute claude --version: ${execError instanceof Error ? execError.message : String(execError)}`
          logger.error('Failed to validate claude binary', execError as Error, {
            claudeBinPath,
            error,
          })
        }
      } else {
        error = `Claude binary not found at ${claudeBinPath}`
      }
    }

    // Check if configDir exists
    const configDirValid = configDir ? existsSync(configDir) : false

    return {
      binDirValid,
      configDirValid,
      claudeVersion,
      error,
    }
  }

  /**
   * Get Claude container mount paths for Docker volume mounting
   */
  getClaudeContainerMounts(): string[] {
    const { claudeConfigDir } = this.getClaudeConfigPaths()
    const mounts: string[] = []

    // Mount projects directory if it exists
    const projectsDir = path.join(claudeConfigDir, 'projects')
    if (existsSync(projectsDir)) {
      mounts.push(`${projectsDir}:/home/ubuntu/.claude/projects`)
    }

    return mounts
  }
}

// Factory function for creating Claude service
export function createClaudeService(settingsService: SettingsService): ClaudeService {
  return new ClaudeService(settingsService)
}
