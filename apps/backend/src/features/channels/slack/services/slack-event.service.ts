import type {
  AskUserQuestionResponse,
  ExitPlanModeResponse,
  Task,
  UserSession,
} from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../../utils/logger'
import { SessionContext } from '../../../auth/services/session-context'
import { RepositoryRepository } from '../../../repositories/db/repository.repository'
import { TaskRepository } from '../../../tasks/db/task.repository'
import {
  PendingResponseService,
  type PendingToolInfo,
} from '../../../tasks/services/pending-response.service'
import { TaskService } from '../../../tasks/services/task.service'
import { WorkspaceRuntimeManager } from '../../../workspace/services/workspace-runtime-manager.service'
import type { SlackThreadProperties } from '../../db/channel-thread.schema'
import { ThreadTrackerService } from '../../services/thread-tracker.service'
import { UserResolverService } from '../../services/user-resolver.service'
import { SlackChannelMappingService } from './slack-channel-mapping.service'
import { SlackMessageService } from './slack-message.service'

/**
 * Slack event payload structure.
 */
export interface SlackEvent {
  type: string
  user: string
  text: string
  ts: string
  channel: string
  thread_ts?: string
  team?: string
}

/**
 * Slack event wrapper with metadata.
 */
export interface SlackEventWrapper {
  type: string
  token?: string
  team_id: string
  api_app_id: string
  event: SlackEvent
  event_id: string
  event_time: number
  challenge?: string
}

/**
 * Result of processing a Slack event.
 */
export interface SlackEventResult {
  success: boolean
  taskId?: string
  isNewTask: boolean
  error?: string
  shouldReply: boolean
  replyMessage?: string
}

/**
 * A message from a Slack thread (from conversations.replies).
 */
export interface SlackThreadMessage {
  user?: string
  text?: string
  ts: string
  bot_id?: string
  username?: string
  blocks?: SlackBlock[]
  attachments?: SlackAttachment[]
}

interface SlackBlock {
  type: string
  text?: { type: string; text: string }
  elements?: Array<{ type: string; elements?: Array<{ type: string; text?: string }> }>
  fields?: Array<{ type: string; text: string }>
}

interface SlackAttachment {
  text?: string
  fallback?: string
  pretext?: string
  title?: string
}

/**
 * Service for processing Slack events.
 * Handles app_mention events to create tasks or send messages.
 */
@injectable()
export class SlackEventService {
  private processedEventIds = new Map<string, number>()
  private readonly EVENT_DEDUP_TTL_MS = 5 * 60 * 1000 // 5 minutes

  constructor(
    @inject(UserResolverService) private userResolver: UserResolverService,
    @inject(ThreadTrackerService) private threadTracker: ThreadTrackerService,
    @inject(TaskService) private taskService: TaskService,
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(RepositoryRepository) private repositoryRepository: RepositoryRepository,
    @inject(SlackChannelMappingService) private channelMappingService: SlackChannelMappingService,
    @inject(SlackMessageService) private slackMessageService: SlackMessageService,
    @inject(PendingResponseService) private pendingResponseService: PendingResponseService,
    @inject(WorkspaceRuntimeManager) private workspaceRuntimeManager: WorkspaceRuntimeManager,
  ) {}

  /**
   * Process a Slack event and send the reply.
   * This is the main entry point called asynchronously from the route handler.
   * Includes deduplication to prevent processing retried events.
   */
  async processAndReply(eventWrapper: SlackEventWrapper): Promise<void> {
    const eventId = eventWrapper.event_id
    if (this.processedEventIds.has(eventId)) {
      logger.info({ eventId }, 'Skipping duplicate Slack event')
      return
    }
    this.processedEventIds.set(eventId, Date.now())
    this.cleanupProcessedEvents()

    const result = await this.processEvent(eventWrapper)

    if (result.shouldReply && result.replyMessage) {
      const event = eventWrapper.event
      if (this.slackMessageService.isEnabled()) {
        await this.slackMessageService.postMessage({
          channel: event.channel,
          threadTs: event.thread_ts || event.ts,
          text: result.replyMessage,
        })
      }
    }
  }

  /**
   * Remove expired event IDs from the deduplication map.
   */
  private cleanupProcessedEvents(): void {
    const now = Date.now()
    for (const [id, timestamp] of this.processedEventIds) {
      if (now - timestamp > this.EVENT_DEDUP_TTL_MS) {
        this.processedEventIds.delete(id)
      }
    }
  }

  /**
   * Process a Slack event.
   *
   * Flow:
   * 1. Handle URL verification challenge
   * 2. For app_mention events:
   *    a. Fetch Slack user email
   *    b. Resolve system user by email
   *    c. Extract repository from message
   *    d. Check if this is a thread reply to existing task
   *    e. Create task or send message
   *
   * @param eventWrapper - The Slack event wrapper
   * @returns Processing result
   */
  async processEvent(eventWrapper: SlackEventWrapper): Promise<SlackEventResult> {
    // Handle URL verification challenge
    if (eventWrapper.type === 'url_verification') {
      return {
        success: true,
        isNewTask: false,
        shouldReply: false,
      }
    }

    const event = eventWrapper.event
    if (!event || event.type !== 'app_mention') {
      return {
        success: true,
        isNewTask: false,
        shouldReply: false,
      }
    }

    logger.info(
      { channel: event.channel, user: event.user, text: event.text?.slice(0, 100) },
      'Processing Slack app_mention event',
    )

    // Immediately react to acknowledge the mention
    await this.slackMessageService.addReaction(
      event.channel,
      event.ts,
      process.env['SLACK_REACTION_PROCESSING'] || 'eyes',
    )

    // 1. Fetch Slack user email and resolve to system user
    const slackUserEmail = await this.fetchSlackUserEmail(event.user, eventWrapper.team_id)
    if (!slackUserEmail) {
      logger.warn({ slackUser: event.user }, 'Could not fetch Slack user email')
      return {
        success: false,
        isNewTask: false,
        shouldReply: true,
        error:
          'Could not identify your Moatless account. Please ensure your Slack email matches your Moatless account email.',
        replyMessage:
          'Could not identify your Moatless account. Please ensure your Slack email matches your Moatless account email.',
      }
    }

    const user = await this.userResolver.resolveByEmail(slackUserEmail)
    if (!user) {
      logger.warn({ email: slackUserEmail }, 'User not found by email')
      return {
        success: false,
        isNewTask: false,
        shouldReply: true,
        error: `No Moatless account found for email ${slackUserEmail}`,
        replyMessage:
          `No Moatless account found for email \`${slackUserEmail}\`.\n\n` +
          'Your Slack email must match your Moatless account email. ' +
          'Please sign up at moatless.xyz using the same email address as your Slack account.',
      }
    }

    logger.info(
      {
        slackUserId: event.user,
        slackEmail: slackUserEmail,
        resolvedUserId: user.id,
        resolvedUserLogin: user.login,
        resolvedUserEmail: user.email,
      },
      'Resolved Moatless user from Slack',
    )

    // Create a synthetic session for the resolved user
    // This enables access control and ownership tracking for task creation
    const syntheticSession: UserSession = {
      id: `slack-${event.user}-${Date.now()}`,
      provider: 'slack',
      userId: user.id,
      userLogin: user.login,
      userEmail: slackUserEmail,
      userName: user.name,
      avatarUrl: user.avatarUrl ?? null,
      role: user.role ?? 'user',
      accessToken: '', // No OAuth token for Slack sessions
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 60 * 60 * 1000), // 1 hour
    }

    // Run the rest of the processing within the user's session context
    try {
      return await SessionContext.run(syntheticSession, () => {
        return this.processEventWithUser(event, eventWrapper, user)
      })
    } catch (error) {
      logger.error(
        { err: error, userId: user.id },
        'Error processing Slack event with user context',
      )
      return {
        success: false,
        isNewTask: false,
        shouldReply: true,
        error: error instanceof Error ? error.message : 'Unknown error',
        replyMessage: `Failed to process your request: ${error instanceof Error ? error.message : 'Unknown error'}. Please try again or contact support.`,
      }
    }
  }

  /**
   * Process event with authenticated user context.
   * Called within SessionContext.run() to ensure proper access control.
   */
  private async processEventWithUser(
    event: SlackEvent,
    eventWrapper: SlackEventWrapper,
    user: {
      id: string
      login: string
      name: string | null
      avatarUrl: string | null
      role?: string
    },
  ): Promise<SlackEventResult> {
    // Identify our own bot's user ID for thread context filtering
    const ownBotUserId = process.env['SLACK_BOT_USER_ID'] || 'U0000000001'

    // 2. Extract repository from message: @moatless repo:my-repo message
    // If no repo: prefix is provided, check for channel default
    const { repository: repositoryName, message } = this.parseMessage(event.text)

    let repository = null

    if (repositoryName) {
      // Explicit repo specified - look up by name and user
      repository = await this.repositoryRepository.findByNameAndUser(repositoryName, user.id)
      if (!repository) {
        return {
          success: false,
          isNewTask: false,
          shouldReply: true,
          error: `Repository '${repositoryName}' not found`,
          replyMessage: `Repository '${repositoryName}' not found. Please check the name and try again.`,
        }
      }
    } else {
      // No repo - check channel default mapping
      repository = await this.channelMappingService.getDefaultRepository(
        event.channel,
        eventWrapper.team_id,
      )
      if (!repository) {
        return {
          success: false,
          isNewTask: false,
          shouldReply: true,
          error: 'No repository specified',
          replyMessage:
            'Please specify a repository: `@moatless repo:my-repo message`\n' +
            'Or ask an admin to configure a default for this channel.',
        }
      }
      logger.info(
        { channelId: event.channel, repositoryId: repository.id },
        'Using channel default repository',
      )
    }

    // 3. Check if this is a reply to an existing task thread
    const threadTs = event.thread_ts || event.ts
    const existingTask = await this.findExistingTask(threadTs)

    if (existingTask) {
      // Check if task has a pending interactive tool request
      const pendingTool = this.pendingResponseService.getPending(existingTask.taskId)

      if (pendingTool) {
        return this.handlePendingToolResponse(existingTask, message, pendingTool, event)
      }

      // No pending interactive tool - send as regular message
      // Fetch thread context for richer agent message
      const threadReplies = await this.fetchThreadReplies(event.channel, threadTs)
      const context = this.formatThreadContext(threadReplies, event.ts, ownBotUserId)
      const composedMessage = this.buildAgentMessage(message, context, false)

      // Send message to existing task
      await this.taskService.sendMessage(
        existingTask.taskId,
        composedMessage,
        true, // resume
        undefined, // model
        undefined, // fileIds
        undefined, // commandName
        undefined, // agentMode
        undefined, // contexts
      )

      // Update thread timestamp and messageTs for reaction tracking
      const thread = await this.threadTracker.findThreadByTaskId(existingTask.taskId, 'slack')
      if (thread?.id) {
        await this.threadTracker.updateLastMessageAt(thread.id)
        const props = thread.properties as SlackThreadProperties
        await this.threadTracker.updateProperties(thread.id, {
          ...props,
          messageTs: event.ts,
        })
      }

      logger.info(
        { taskId: existingTask.taskId, channel: event.channel },
        'Message sent to existing task from Slack',
      )

      return {
        success: true,
        taskId: existingTask.taskId,
        isNewTask: false,
        shouldReply: false,
      }
    }

    // 4. Create new task with thread context
    const threadReplies = await this.fetchThreadReplies(event.channel, threadTs)
    const context = this.formatThreadContext(threadReplies, event.ts, ownBotUserId)
    const composedMessage = this.buildAgentMessage(message, context, true)

    const task = await this.taskService.createTask({
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: composedMessage,
      start: true,
      tags: ['slack'],
    })

    // Create Slack thread for tracking
    const threadProperties: SlackThreadProperties = {
      channelId: event.channel,
      teamId: eventWrapper.team_id,
      userId: event.user,
      messageTs: event.ts,
    }

    await this.threadTracker.createSlackThread(task.taskId, threadTs, threadProperties)

    const taskUrl = this.getTaskUrl(task.taskId)

    logger.info({ taskId: task.taskId, channel: event.channel }, 'New task created from Slack')

    return {
      success: true,
      taskId: task.taskId,
      isNewTask: true,
      shouldReply: true,
      replyMessage: `Task created! \n${taskUrl}`,
    }
  }

  /**
   * Parse the message text to extract repository and message.
   * Format: @moatless repo:my-repo message here
   */
  private parseMessage(text: string): { repository: string | null; message: string } {
    // Remove the @mention at the start
    const cleanText = text.replace(/^<@[A-Z0-9]+>\s*/i, '').trim()

    // Look for repo:name pattern
    const repoMatch = cleanText.match(/\brepo:([a-z0-9_-]+)/i)
    const repository = repoMatch?.[1]?.toLowerCase() || null

    // Remove repo: pattern from message
    const message = cleanText.replace(/\brepo:[a-z0-9_-]+\s*/i, '').trim()

    return { repository, message }
  }

  /**
   * Find an existing task from Slack thread.
   */
  private async findExistingTask(threadTs: string): Promise<Task | null> {
    const thread = await this.threadTracker.findSlackThread(threadTs)
    if (!thread) {
      return null
    }

    const task = await this.taskRepository.get(thread.taskId)
    if (task) {
      logger.debug({ threadTs }, 'Found task via Slack thread')
      return task as Task
    }

    return null
  }

  /**
   * Fetch Slack user's email address via Slack API.
   * Requires users:read.email scope.
   */
  private async fetchSlackUserEmail(userId: string, _teamId: string): Promise<string | null> {
    const botToken = process.env['SLACK_BOT_TOKEN']
    if (!botToken) {
      logger.warn('SLACK_BOT_TOKEN not configured - cannot fetch user email')
      return null
    }

    try {
      const response = await fetch(`https://slack.com/api/users.info?user=${userId}`, {
        headers: {
          Authorization: `Bearer ${botToken}`,
        },
      })

      const data = (await response.json()) as {
        ok: boolean
        user?: { profile?: { email?: string } }
        error?: string
      }

      if (!data.ok) {
        logger.warn({ error: data.error, userId }, 'Slack API error fetching user info')
        return null
      }

      return data.user?.profile?.email || null
    } catch (error) {
      logger.error({ err: error, userId }, 'Error fetching Slack user email')
      return null
    }
  }

  /**
   * Fetch thread replies from Slack conversations.replies API.
   * Returns empty array on failure (graceful degradation).
   */
  private async fetchThreadReplies(
    channel: string,
    threadTs: string,
  ): Promise<SlackThreadMessage[]> {
    const botToken = process.env['SLACK_BOT_TOKEN']
    if (!botToken) {
      return []
    }

    try {
      const url = new URL('https://slack.com/api/conversations.replies')
      url.searchParams.set('channel', channel)
      url.searchParams.set('ts', threadTs)
      url.searchParams.set('limit', '50')

      const response = await fetch(url.toString(), {
        headers: { Authorization: `Bearer ${botToken}` },
      })

      const data = (await response.json()) as {
        ok: boolean
        messages?: SlackThreadMessage[]
        error?: string
      }

      if (!data.ok) {
        logger.warn({ channel, threadTs, error: data.error }, 'Failed to fetch thread replies')
        return []
      }

      return data.messages || []
    } catch (error) {
      logger.warn({ err: error, channel, threadTs }, 'Error fetching thread replies')
      return []
    }
  }

  /**
   * Extract displayable text from a Slack message, falling back through text → blocks → attachments.
   */
  private extractMessageText(msg: SlackThreadMessage): string {
    if (msg.text) {
      return msg.text
    }

    if (msg.blocks && msg.blocks.length > 0) {
      const blockText = this.extractBlocksText(msg.blocks)
      if (blockText) {
        return blockText
      }
    }

    if (msg.attachments && msg.attachments.length > 0) {
      const attachmentText = this.extractAttachmentsText(msg.attachments)
      if (attachmentText) {
        return attachmentText
      }
    }

    return '[no text content]'
  }

  private extractBlocksText(blocks: SlackBlock[]): string | null {
    const texts: string[] = []
    for (const block of blocks) {
      if (block.text?.text) {
        texts.push(block.text.text)
      }
      if (block.fields) {
        for (const field of block.fields) {
          if (field.text) {
            texts.push(field.text)
          }
        }
      }
      if (block.type === 'rich_text' && block.elements) {
        this.extractRichTextElements(block.elements, texts)
      }
    }
    return texts.length > 0 ? texts.join(' ') : null
  }

  private extractRichTextElements(
    elements: Array<{ type: string; elements?: Array<{ type: string; text?: string }> }>,
    texts: string[],
  ): void {
    for (const section of elements) {
      if (section.elements) {
        for (const el of section.elements) {
          if (el.text) {
            texts.push(el.text)
          }
        }
      }
    }
  }

  private extractAttachmentsText(attachments: SlackAttachment[]): string | null {
    const parts: string[] = []
    for (const att of attachments) {
      if (att.title) {
        parts.push(att.title)
      }
      if (att.text) {
        parts.push(att.text)
      } else if (att.fallback) {
        parts.push(att.fallback)
      }
      if (att.pretext) {
        parts.push(att.pretext)
      }
    }
    return parts.length > 0 ? parts.join(' | ') : null
  }

  /**
   * Format a Slack timestamp to human-readable format.
   * Slack ts format: "1234567890.123456" (Unix seconds with microseconds)
   * Returns format: "Jan 26 14:32"
   */
  private formatSlackTimestamp(ts: string): string {
    const unixSeconds = Number.parseFloat(ts)
    const date = new Date(unixSeconds * 1000)

    const month = date.toLocaleDateString('en-US', { month: 'short' })
    const day = date.getDate()
    const hours = date.getHours().toString().padStart(2, '0')
    const minutes = date.getMinutes().toString().padStart(2, '0')

    return `${month} ${day} ${hours}:${minutes}`
  }

  /**
   * Format thread context from replies.
   * Finds the last message from our own bot (by user ID), includes messages after it.
   * Messages from other bots are preserved as context.
   * Filters out the triggering message and our own bot's messages.
   * Truncates to last 20 messages and ~4000 chars.
   */
  private formatThreadContext(
    replies: SlackThreadMessage[],
    triggeringTs: string,
    ownBotUserId: string | null,
  ): string | null {
    if (replies.length === 0) {
      return null
    }

    // Find the last message from OUR bot (by user ID, not any bot_id)
    let lastOwnBotIndex = -1
    for (let i = replies.length - 1; i >= 0; i--) {
      if (ownBotUserId && replies[i]?.user === ownBotUserId) {
        lastOwnBotIndex = i
        break
      }
    }

    // Include messages after our bot's last reply (or all if no own bot messages)
    const startIndex = lastOwnBotIndex >= 0 ? lastOwnBotIndex + 1 : 0
    let contextMessages = replies.slice(startIndex)

    // Filter out the triggering message and our own bot's messages only
    contextMessages = contextMessages.filter(
      (msg) => msg.ts !== triggeringTs && msg.user !== ownBotUserId,
    )

    if (contextMessages.length === 0) {
      return null
    }

    // Truncate to last 20 messages
    if (contextMessages.length > 20) {
      contextMessages = contextMessages.slice(-20)
    }

    // Format as [userId]: text lines, truncate total to ~4000 chars
    const lines: string[] = []
    let totalLength = 0
    const maxLength = 4000

    for (const msg of contextMessages) {
      const sender = msg.user || msg.username || msg.bot_id || 'bot'
      const text = this.extractMessageText(msg)
      const timestamp = this.formatSlackTimestamp(msg.ts)
      const line = `[${timestamp}] [${sender}]: ${text}`
      if (totalLength + line.length > maxLength) {
        break
      }
      lines.push(line)
      totalLength += line.length + 1 // +1 for newline
    }

    return lines.length > 0 ? lines.join('\n') : null
  }

  /**
   * Build the full agent message with thread context.
   */
  private buildAgentMessage(
    instruction: string,
    context: string | null,
    isNewTask: boolean,
  ): string {
    const formatInstructions = isNewTask
      ? [
          '',
          '--- Response format ---',
          'Your response will be posted to Slack. Use Slack mrkdwn formatting:',
          '- Use *bold* (single asterisks) instead of **bold**',
          '- No ## headers — use *Bold Title* on its own line for section headers',
          '- Links: <url|text> instead of [text](url)',
          '- Keep responses concise and scannable',
          '- Code blocks with ``` work normally',
        ].join('\n')
      : ''

    if (!context) {
      return `[Source: Slack]\n\n${instruction}${formatInstructions}`
    }

    const contextHeader = isNewTask
      ? '--- Thread context (prior conversation) ---'
      : '--- Messages since last agent reply ---'
    const contextFooter = isNewTask ? '--- End thread context ---' : '--- End messages ---'

    const parts = [
      '[Source: Slack thread]',
      '',
      contextHeader,
      context,
      contextFooter,
      '',
      '--- Instruction ---',
      instruction,
    ]
    if (formatInstructions) {
      parts.push(formatInstructions)
    }
    return parts.join('\n')
  }

  /**
   * Get the frontend URL for a task.
   */
  private getTaskUrl(taskId: string): string {
    const frontendUrl = (process.env['FRONTEND_URL'] || 'http://localhost:8000').replace(/\/$/, '')
    return `${frontendUrl}/tasks/${taskId}`
  }

  /**
   * Handle a pending tool response from a Slack thread reply.
   * Extracted to reduce cognitive complexity.
   */
  private async handlePendingToolResponse(
    existingTask: Task,
    message: string,
    pendingTool: PendingToolInfo,
    _event: SlackEvent,
  ): Promise<SlackEventResult> {
    const result = await this.submitUserInputResponse(existingTask.taskId, message, pendingTool)

    // Update thread timestamp
    const thread = await this.threadTracker.findThreadByTaskId(existingTask.taskId, 'slack')
    if (thread?.id) {
      await this.threadTracker.updateLastMessageAt(thread.id)
      // Remove question reaction since user has responded
      const props = thread.properties as SlackThreadProperties
      if (props.messageTs) {
        await this.slackMessageService.removeReaction(props.channelId, props.messageTs, 'question')
      }
    }

    if (!result.success) {
      const errorMessage = result.error || 'Failed to submit response'
      logger.error(
        { taskId: existingTask.taskId, error: errorMessage },
        'Failed to submit user input response from Slack',
      )
      return {
        success: false,
        taskId: existingTask.taskId,
        isNewTask: false,
        error: errorMessage,
        shouldReply: true,
        replyMessage: `Failed to submit response: ${errorMessage}`,
      }
    }

    logger.info(
      { taskId: existingTask.taskId, toolName: pendingTool.toolName },
      'User input response submitted from Slack',
    )

    return {
      success: true,
      taskId: existingTask.taskId,
      isNewTask: false,
      shouldReply: false,
    }
  }

  /**
   * Submit a user input response for an interactive tool (AskUserQuestion or ExitPlanMode).
   * Parses the user's Slack message and routes it to the sandbox.
   */
  private async submitUserInputResponse(
    taskId: string,
    message: string,
    pendingTool: PendingToolInfo,
  ): Promise<{ success: boolean; error?: string }> {
    try {
      // Parse the user's message into the appropriate response format
      const response = this.parseUserInputResponse(message, pendingTool)

      // Get sandbox URL
      const sandboxUrl = await this.workspaceRuntimeManager.getSandboxUrl(taskId)
      if (!sandboxUrl) {
        return { success: false, error: 'Sandbox URL not available' }
      }

      // Send to sandbox
      const proxyResponse = await fetch(`${sandboxUrl}/user-input-response`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          toolName: pendingTool.toolName,
          response,
        }),
      })

      const result = (await proxyResponse.json()) as { success: boolean; error?: string }

      if (!proxyResponse.ok) {
        return { success: false, error: result.error || 'Sandbox error' }
      }

      // Clear pending state on success
      this.pendingResponseService.clearPending(taskId)

      return { success: true }
    } catch (error) {
      logger.error(
        { err: error, taskId, toolName: pendingTool.toolName },
        'Failed to submit user input response',
      )
      return { success: false, error: error instanceof Error ? error.message : 'Unknown error' }
    }
  }

  /**
   * Parse a user's Slack message into the appropriate response format for the pending tool.
   */
  private parseUserInputResponse(
    message: string,
    pendingTool: PendingToolInfo,
  ): AskUserQuestionResponse | ExitPlanModeResponse {
    const text = message.trim().toLowerCase()

    if (pendingTool.toolName === 'ExitPlanMode') {
      // Check for approval keywords
      const approvalKeywords = [
        'approve',
        'approved',
        'yes',
        'proceed',
        'ok',
        'lgtm',
        'go ahead',
        'looks good',
      ]
      const isApproved = approvalKeywords.some(
        (keyword) => text === keyword || text.startsWith(keyword),
      )

      if (isApproved) {
        return { approved: true }
      }
      // Treat as feedback
      return { approved: false, feedback: message.trim() }
    }

    // AskUserQuestion - treat the message as the answer to the first question (index "0")
    // For multiple questions, users would need to use the UI
    const response: AskUserQuestionResponse = {
      answers: { '0': message.trim() },
    }

    return response
  }
}
