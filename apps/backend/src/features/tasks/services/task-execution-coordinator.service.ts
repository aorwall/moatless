import { randomUUID } from 'node:crypto'
import type {
  AskUserQuestionResponse,
  ExitPlanModeResponse,
  MessageContentBlock,
  MessageContext,
  TaskTurnEvent,
} from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { AgentRequestBuilder } from '../../agents/services/agent-request-builder.service'
import { CommandService } from '../../agents/services/command.service'
import { MessageService } from '../../agents/services/message.service'
import { AgentRequestBuilderSymbol } from '../../agents/symbols'
import { runInBackgroundWithSession } from '../../auth'
import { EventService } from '../../events/index'
import { agentExecutionTotal } from '../../metrics'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../db/task.repository'

/**
 * TaskExecutionCoordinator - Orchestrates agent execution and workspace management
 *
 * Handles:
 * - Starting tasks and turns
 * - Sending messages and commands
 * - Workspace provisioning and validation
 * - Notifying sandbox of new messages (sandbox polls backend API for actual data)
 * - Resume logic for continuing from previous turns
 *
 * Message delivery is DB-based: messages are saved to the database, then a
 * lightweight push notification wakes the sandbox to poll for pending messages.
 *
 * @see docs/architecture/tasks-feature.md
 */
@injectable()
export class TaskExecutionCoordinator {
  private static readonly WORKSPACE_READY_TIMEOUT = 30000 // 30 seconds
  private static readonly WORKSPACE_CHECK_INTERVAL = 500 // 500ms

  constructor(
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(WorkspaceRuntimeManager) private readonly workspaceCoordinator: WorkspaceRuntimeManager,
    @inject(EventService) private readonly eventService: EventService,
    @inject(CommandService) private readonly commandService: CommandService,
    @inject(MessageService) private readonly messageService: MessageService,
    @inject(AgentRequestBuilderSymbol) private readonly agentRequestBuilder: AgentRequestBuilder,
  ) {}

  /**
   * Resume task execution from a previous agent session.
   * Used for continuing an interrupted task.
   *
   * Behavior:
   * - WITH agentSessionId: Resume using SDK session resume feature (sends empty message)
   * - WITHOUT agentSessionId: Start fresh (can't resume SDK session)
   *
   * @param taskId - Task ID
   * @param agentSessionId - The agent session ID to resume
   */
  async resumeTask(taskId: string, agentSessionId: string): Promise<void> {
    const taskLogger = logger.child({
      taskId,
      agentSessionId,
      handler: 'resumeTask',
    })

    // Get task for repository ID and validation
    const task = await this.taskRepository.getInternal(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Validate task is in a resumable state
    if (task.status !== 'running') {
      throw new ValidationError(
        `Cannot resume task with status "${task.status}". Only "running" tasks can be resumed.`,
        { status: task.status },
      )
    }

    taskLogger.info('Resuming task with SDK session resume')

    // Notify sandbox to pick up the pending messages
    // Fire-and-forget — sandbox will poll on its own timeout anyway
    this.workspaceCoordinator.notifySandbox(taskId).catch((err) => {
      taskLogger.debug({ err }, 'Failed to notify sandbox for resume (best-effort)')
    })

    // Ensure workspace is ready (non-blocking)
    this.ensureWorkspaceInBackground(taskId, task.repositoryId)

    agentExecutionTotal.labels({ status: 'queued', agent_type: 'claude-code' }).inc()
  }

  /**
   * Send a command to a task (command prompt becomes the message).
   *
   * @param taskId - Task ID
   * @param commandName - Name of command to execute
   * @param resume - Whether to resume from previous turn (default: true)
   * @param model - Optional model override
   * @param fileIds - Optional file attachments
   */
  async sendCommand(
    taskId: string,
    commandName: string,
    resume = true,
    model?: string,
    fileIds?: string[],
  ): Promise<void> {
    const command = await this.commandService.getCommand(commandName)
    if (!command) {
      throw new ValidationError(`Command '${commandName}' not found`, 'commandName')
    }

    // Use the command's prompt as the message
    await this.sendMessage(
      taskId,
      command.prompt,
      resume,
      model,
      fileIds,
      commandName,
      command.agentMode || 'edit',
    )
  }

  /**
   * Send a message to a task, creating a new turn and notifying sandbox.
   *
   * Message Persistence Flow:
   * 1. Message is saved to MessageService FIRST (agentSessionId=null, sentAt=null)
   * 2. This ensures message is immediately available via GET /messages (with isQueued=true)
   * 3. Backend notifies sandbox via POST /notify-messages (best-effort push)
   * 4. Sandbox polls GET /messages/pending to fetch unsent messages
   * 5. When agent emits first user/system event, message is marked as acknowledged (sentAt set)
   *
   * @param taskId - Task ID
   * @param message - User message
   * @param resume - Whether to resume from previous turn
   * @param model - Optional model override
   * @param fileIds - Optional file attachments
   * @param commandName - Optional command name
   * @param agentMode - Agent mode (edit/plan)
   * @param contexts - Optional message contexts
   */
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex orchestration logic necessary for agent execution
  async sendMessage(
    taskId: string,
    message: string,
    resume = true,
    model?: string,
    fileIds?: string[],
    _commandName?: string,
    agentMode?: 'edit' | 'plan',
    contexts?: MessageContext[],
    toolUseId?: string,
    toolName?: 'AskUserQuestion' | 'ExitPlanMode',
    toolResponse?: AskUserQuestionResponse | ExitPlanModeResponse,
  ): Promise<void> {
    const taskLogger = logger.child({ taskId, handler: 'sendMessage' })

    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Look up the resume session ID when resume is requested
    let resumeSessionId: string | undefined
    if (resume) {
      resumeSessionId = await this.messageService.getLatestAgentSessionId(taskId)
      taskLogger.debug({ resumeSessionId: resumeSessionId ?? null }, 'Looked up resume session ID')
    }

    // Save message based on type
    if (toolUseId && toolName && toolResponse) {
      // Interactive tool response: save as tool_result message
      await this.messageService.saveUserInputAsToolResult(taskId, toolName, toolUseId, toolResponse)
      taskLogger.info({ toolName, toolUseId }, 'Tool result saved to DB')
    } else {
      // Build enriched request via AgentRequestBuilder to get content blocks + system prompt
      const enrichedRequest = await this.agentRequestBuilder.buildRequest(
        taskId,
        message,
        0,
        agentMode,
        model,
        task.repositoryId,
        { resumeSessionId, fileIds, contexts, taskName: task.name },
      )

      // Build content blocks with showInUI flags from the enriched request
      const contentBlocks = this.buildContentBlocks(message, enrichedRequest)

      await this.messageService.saveUserMessage(
        taskId,
        message,
        undefined, // sessionId - not known yet
        undefined, // turnNumber - no longer tracked
        fileIds,
        contexts,
        agentMode,
        model,
        resumeSessionId,
        contentBlocks,
      )
      taskLogger.info(
        {
          hasContexts: (contexts?.length ?? 0) > 0,
          hasContentBlocks: contentBlocks.length,
          resumeSessionId: resumeSessionId ?? null,
        },
        'User message saved to DB with enriched content blocks',
      )
    }

    // Update task metadata
    const now = new Date()
    task.status = 'running'
    task.lastMessageAt = now

    // Save task with updated metadata
    await this.taskRepository.save(task)

    // Notify sandbox that new messages are available (fire-and-forget)
    this.workspaceCoordinator.notifySandbox(taskId).catch((err) => {
      taskLogger.debug({ err }, 'Failed to notify sandbox (best-effort)')
    })

    taskLogger.info('Message saved and sandbox notified')

    // Ensure workspace is provisioning in background (non-blocking)
    this.ensureWorkspaceInBackground(taskId, task.repositoryId)

    agentExecutionTotal.labels({ status: 'queued', agent_type: 'claude-code' }).inc()
  }

  /**
   * Start a task by notifying the sandbox to pick up the initial message.
   *
   * This method:
   * 1. Validates task can be started
   * 2. Waits for workspace to be ready
   * 3. Updates task status to 'running' and emits event
   * 4. Notifies sandbox to poll for pending messages
   *
   * @param taskId - Task ID to start
   * @param fileIds - Optional file attachments
   * @param contexts - Optional message contexts
   * @param _waitForFirstEvent - Deprecated, no longer used (events come via Redis subscription)
   */
  async startTask(
    taskId: string,
    fileIds?: string[],
    contexts?: MessageContext[],
    _waitForFirstEvent = false,
  ): Promise<void> {
    const taskLogger = logger.child({ taskId, handler: 'startTask' })

    taskLogger.info(
      {
        hasFileIds: (fileIds?.length ?? 0) > 0,
        hasContexts: (contexts?.length ?? 0) > 0,
      },
      'Starting task',
    )

    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Validate task can be started
    if (task.status !== 'open' && task.status !== 'error') {
      throw new ValidationError(
        `Cannot start task with status "${task.status}". Only "open" or "error" tasks can be started.`,
        { status: task.status },
      )
    }

    // Wait for workspace to be ready by checking snapshot
    await this.waitForWorkspaceReady(taskId)

    // Update task status to running
    task.status = 'running'

    // Save task with updated status
    await this.taskRepository.save(task)

    // Emit turn active event
    const now = new Date()
    await this.eventService.emitEvent({
      taskId,
      type: 'taskTurn',
      subType: 'active',
      agentType: task.agentType,
      turnNumber: 0,
      payload: {
        status: 'active',
        previousStatus: 'open',
        turnNumber: 0,
      },
      eventId: randomUUID(),
      timestamp: now,
      isError: false,
    } as TaskTurnEvent)

    // Notify sandbox to poll for pending messages (fire-and-forget)
    this.workspaceCoordinator.notifySandbox(taskId).catch((err) => {
      taskLogger.debug({ err }, 'Failed to notify sandbox for start (best-effort)')
    })

    taskLogger.info({}, 'Task started, sandbox notified')

    agentExecutionTotal.labels({ status: 'queued', agent_type: 'claude-code' }).inc()

    taskLogger.debug({}, 'Task started successfully')
  }

  /**
   * Ensure workspace is provisioning in the background (non-blocking).
   * This is called after saving messages to ensure workspace starts setting up.
   *
   * The method:
   * 1. Provisions workspace (saves 'Pending' snapshot immediately for polling)
   * 2. Updates task with workspace path when ready
   *
   * Agent execution is decoupled - the sandbox polls for pending messages
   * and executes the agent once the workspace is ready.
   *
   * Errors are saved in the workspace snapshot and can be retrieved via workspace-status API.
   *
   * @param taskId - Task ID
   * @param repositoryId - Repository ID for workspace setup
   */
  ensureWorkspaceInBackground(taskId: string, repositoryId: string): void {
    // Run in background with session context preserved for access control
    runInBackgroundWithSession(async () => {
      try {
        logger.debug({ taskId, repositoryId }, 'Starting background workspace setup')

        // Provision workspace (saves 'Pending' snapshot first for frontend polling)
        const handle = await this.workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

        logger.info({ taskId, workspacePath: handle.workspacePath }, 'Workspace provisioned')
      } catch (error) {
        // Error is already saved in snapshot by ensureKubernetesWorkspace
        // Just log here for visibility
        logger.error(
          { err: error as Error, taskId, repositoryId },
          'Background workspace setup failed',
        )
      }
    })
  }

  // ========================================
  // Private helper methods
  // ========================================

  /**
   * Build content blocks from the enriched AgentRunRequest.
   *
   * The original user text gets showInUI=true. All other enrichments
   * (file list headers, context sections, images, system prompts)
   * get showInUI=false so the frontend only shows the raw user message.
   */
  private buildContentBlocks(
    originalMessage: string,
    enrichedRequest: import('@moatless/sandbox-schemas').AgentRunRequest,
  ): MessageContentBlock[] {
    const blocks: MessageContentBlock[] = []
    let sortOrder = 0

    // Original user text — visible in UI
    blocks.push({
      sortOrder: sortOrder++,
      blockType: 'text',
      showInUI: true,
      textContent: originalMessage,
    })

    // Enriched content blocks from AgentRequestBuilder (agent-only)
    for (const contentItem of enrichedRequest.content) {
      if (contentItem.type === 'text') {
        // Skip the first text block if it starts with the original message
        // (AgentRequestBuilder prepends file info + original message as the first text block)
        if (blocks.length === 1 && contentItem.text.endsWith(originalMessage)) {
          // Extract the prepended enrichment (file list header, etc.)
          const enrichment = contentItem.text.slice(
            0,
            contentItem.text.length - originalMessage.length,
          )
          if (enrichment.trim()) {
            blocks.push({
              sortOrder: sortOrder++,
              blockType: 'text',
              showInUI: false,
              textContent: enrichment.trim(),
            })
          }
        } else if (contentItem.text !== originalMessage) {
          // Additional text content (context sections, etc.)
          blocks.push({
            sortOrder: sortOrder++,
            blockType: 'text',
            showInUI: false,
            textContent: contentItem.text,
          })
        }
      } else if (contentItem.type === 'image') {
        blocks.push({
          sortOrder: sortOrder++,
          blockType: 'image',
          showInUI: false,
          imageMediaType: contentItem.source.media_type,
          imageData: contentItem.source.data,
        })
      } else if (contentItem.type === 'tool_result') {
        blocks.push({
          sortOrder: sortOrder++,
          blockType: 'tool_result',
          showInUI: false,
          toolUseId: contentItem.tool_use_id,
          toolResultContent: contentItem.content,
          toolResultIsError: contentItem.is_error,
        })
      }
    }

    // appendSystemPrompt → stored as a 'system' content block
    if (enrichedRequest.appendSystemPrompt) {
      blocks.push({
        sortOrder: sortOrder++,
        blockType: 'system',
        showInUI: false,
        textContent: enrichedRequest.appendSystemPrompt,
      })
    }

    return blocks
  }

  /**
   * Wait for workspace to be ready by polling snapshots.
   */
  private async waitForWorkspaceReady(taskId: string): Promise<void> {
    const maxWaitTime = TaskExecutionCoordinator.WORKSPACE_READY_TIMEOUT
    const checkInterval = TaskExecutionCoordinator.WORKSPACE_CHECK_INTERVAL
    const startWait = Date.now()

    while (Date.now() - startWait < maxWaitTime) {
      const record = await this.workspaceCoordinator.getSandboxRecord(taskId)
      if (record) {
        // Unified availability check for both local and Kubernetes workspaces
        if (await this.workspaceCoordinator.isWorkspaceAvailable(taskId)) {
          return
        }
        // Check for errors in execution target
        if (record.executionTarget.lastError) {
          throw new Error(
            `Cannot start task - workspace setup failed: ${record.executionTarget.lastError}`,
          )
        }
      }
      await new Promise((resolve) => setTimeout(resolve, checkInterval))
    }

    throw new Error('Workspace ready timeout')
  }
}
