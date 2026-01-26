import type { AskUserQuestionResponse, ExitPlanModeResponse, Task } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Attachment } from 'mailparser'
import { logger } from '../../../../utils/logger'
import { createSyntheticSession, SessionContext } from '../../../auth'
import { FileStorageService } from '../../../file-storage'
import { RepositoryRepository } from '../../../repositories/db/repository.repository'
import { TaskRepository } from '../../../tasks/db/task.repository'
import {
  PendingResponseService,
  type PendingToolInfo,
} from '../../../tasks/services/pending-response.service'
import { TaskService } from '../../../tasks/services/task.service'
import { WorkspaceRuntimeManager } from '../../../workspace/services/workspace-runtime-manager.service'
import { getSandboxUrl } from '../../../workspace/types/workspace-runtime.types'
import type { EmailThreadProperties } from '../../db/channel-thread.schema'
import { ThreadTrackerService } from '../../services/thread-tracker.service'
import { UserResolverService } from '../../services/user-resolver.service'
import { extractShortIdFromSubject } from '../../utils/short-id.utils'
import {
  cleanEmailBody,
  extractEmailAddress,
  parseRecipientAddress,
} from '../utils/email-parser.utils'
import { EmailOutboundService } from './email-outbound.service'

/**
 * Parsed email data from SendGrid webhook.
 */
export interface InboundEmail {
  to: string
  from: string
  subject: string
  text: string
  html?: string
  messageId: string
  inReplyTo?: string
  references?: string
  attachments?: Attachment[]
}

/**
 * Result of processing an inbound email.
 */
export interface EmailProcessingResult {
  success: boolean
  taskId?: string
  isNewTask: boolean
  error?: string
}

/**
 * Service for processing inbound emails from SendGrid.
 * Handles email parsing, user resolution, and task creation/messaging.
 */
@injectable()
export class EmailInboundService {
  constructor(
    @inject(UserResolverService) private userResolver: UserResolverService,
    @inject(ThreadTrackerService) private threadTracker: ThreadTrackerService,
    @inject(TaskService) private taskService: TaskService,
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(RepositoryRepository) private repositoryRepository: RepositoryRepository,
    @inject(EmailOutboundService) private emailOutbound: EmailOutboundService,
    @inject(FileStorageService) private fileStorageService: FileStorageService,
    @inject(PendingResponseService) private pendingResponseService: PendingResponseService,
    @inject(WorkspaceRuntimeManager) private workspaceRuntimeManager: WorkspaceRuntimeManager,
  ) {}

  /**
   * Process an inbound email from SendGrid.
   *
   * Flow:
   * 1. Parse recipient to extract repository name
   * 2. Resolve user by sender email address
   * 3. Find repository by name and user
   * 4. Check if this is a reply (threading) or new task
   * 5. Create task or send message to existing task
   *
   * @param email - The parsed email data
   * @returns Processing result
   */
  async processEmail(email: InboundEmail): Promise<EmailProcessingResult> {
    logger.info(
      {
        to: email.to,
        from: email.from,
        subject: email.subject,
        attachmentCount: email.attachments?.length ?? 0,
      },
      'Processing inbound email',
    )

    const senderEmail = extractEmailAddress(email.from)

    // 1. Parse recipient address to get repository name
    const repositoryName = parseRecipientAddress(email.to)
    if (!repositoryName) {
      logger.warn({ to: email.to }, 'Invalid recipient address format')
      const error = 'Invalid recipient address format. Expected: {repository}@inbox.moatless.ai'
      await this.sendErrorReply(email, senderEmail, error)
      return { success: false, isNewTask: false, error }
    }

    // 2. Resolve user by sender email address
    const user = await this.userResolver.resolveByEmail(senderEmail)
    if (!user) {
      logger.warn({ senderEmail }, 'User not found by email')
      const error = `No user found with email '${senderEmail}'. Please sign up first.`
      await this.sendErrorReply(email, senderEmail, error)
      return { success: false, isNewTask: false, error }
    }

    // 3. Find repository by name and user
    const repository = await this.repositoryRepository.findByNameAndUser(repositoryName, user.id)
    if (!repository) {
      logger.warn({ repositoryName, userId: user.id }, 'Repository not found')
      const error = `Repository '${repositoryName}' not found. Check the email address and try again.`
      await this.sendErrorReply(email, senderEmail, error)
      return { success: false, isNewTask: false, error }
    }

    // Clean the email body
    const message = cleanEmailBody(email.text)

    if (!message) {
      logger.warn(
        { repositoryName, userId: user.id, senderEmail, body: email.text },
        'No content in email body, aborting...',
      )
      return { success: false, isNewTask: false, error: 'No message in email' }
    }

    // Process attachments and get fileIds
    const fileIds = await this.processAttachments(email.attachments)

    // 4. Run task operations within the user's session context
    // This ensures createdBy is set correctly when creating tasks
    return SessionContext.run(createSyntheticSession(user, 'email'), async () => {
      // Check if this is a reply to an existing task
      const existingTask = await this.findExistingTask(email)

      if (existingTask) {
        // Check if task has a pending interactive tool request
        const pendingTool = this.pendingResponseService.getPending(existingTask.taskId)

        if (pendingTool) {
          return this.handlePendingToolResponse(existingTask, message, pendingTool)
        }

        // No pending interactive tool - send as regular message (with attachments if any)
        await this.taskService.sendMessage(
          existingTask.taskId,
          message,
          true, // resume
          undefined, // model
          fileIds.length > 0 ? fileIds : undefined, // fileIds
          undefined, // commandName
          undefined, // agentMode
          undefined, // contexts
        )

        // Update thread timestamp
        const thread = await this.threadTracker.findThreadByTaskId(existingTask.taskId, 'email')
        if (thread?.id) {
          await this.threadTracker.updateLastMessageAt(thread.id)
        }

        logger.info(
          { taskId: existingTask.taskId, from: email.from, attachmentCount: fileIds.length },
          'Message sent to existing task from email',
        )

        return {
          success: true,
          taskId: existingTask.taskId,
          isNewTask: false,
        }
      }

      // 5. Create new task with attachments
      const task = await this.taskService.createTask({
        agentType: 'claude-code',
        repositoryId: repository.id,
        message,
        fileIds: fileIds.length > 0 ? fileIds : undefined,
        start: true,
      })

      // Create email thread for tracking
      const threadProperties: EmailThreadProperties = {
        recipientEmail: extractEmailAddress(email.from),
        replyFromEmail: extractEmailAddress(email.to),
        initialSubject: email.subject,
        fromEmail: extractEmailAddress(email.from),
      }

      await this.threadTracker.createEmailThread(task.taskId, email.messageId, threadProperties)

      logger.info(
        { taskId: task.taskId, from: email.from, attachmentCount: fileIds.length },
        'New task created from email',
      )

      return {
        success: true,
        taskId: task.taskId,
        isNewTask: true,
      }
    })
  }

  /**
   * Process email attachments and save them to file storage.
   *
   * @param attachments - The email attachments from mailparser
   * @returns Array of file IDs for the saved attachments
   */
  private async processAttachments(attachments?: Attachment[]): Promise<string[]> {
    if (!attachments || attachments.length === 0) {
      return []
    }

    const fileIds: string[] = []
    for (const attachment of attachments) {
      try {
        const uploaded = await this.fileStorageService.uploadFile(
          attachment.content,
          attachment.filename || 'attachment',
          attachment.contentType || 'application/octet-stream',
        )
        fileIds.push(uploaded.fileId)
        logger.debug(
          { filename: attachment.filename, fileId: uploaded.fileId },
          'Saved email attachment',
        )
      } catch (error) {
        logger.error({ err: error, filename: attachment.filename }, 'Failed to save attachment')
      }
    }
    return fileIds
  }

  /**
   * Find an existing task from email threading headers or subject.
   *
   * Priority:
   * 1. In-Reply-To header → find thread by Message-ID
   * 2. References header → check each reference
   * 3. Subject line [TASK-abc123] → find by shortId
   *
   * @param email - The inbound email
   * @returns The task if found, null otherwise
   */
  private async findExistingTask(email: InboundEmail): Promise<Task | null> {
    // 1. Check In-Reply-To header
    const taskFromReplyTo = await this.findTaskByMessageId(email.inReplyTo, 'In-Reply-To')
    if (taskFromReplyTo) {
      return taskFromReplyTo
    }

    // 2. Check References header (space-separated list of Message-IDs)
    const taskFromRefs = await this.findTaskFromReferences(email.references)
    if (taskFromRefs) {
      return taskFromRefs
    }

    // 3. Check subject for [TASK-abc123] pattern
    return this.findTaskBySubjectShortId(email.subject)
  }

  /**
   * Find a task by email Message-ID.
   */
  private async findTaskByMessageId(
    messageId: string | undefined,
    headerName: string,
  ): Promise<Task | null> {
    if (!messageId) {
      return null
    }
    const thread = await this.threadTracker.findEmailThread(messageId)
    if (!thread) {
      return null
    }
    const task = await this.taskRepository.get(thread.taskId)
    if (task) {
      logger.debug({ messageId }, `Found task via ${headerName} header`)
      return task as Task
    }
    return null
  }

  /**
   * Find a task from References header.
   */
  private async findTaskFromReferences(references: string | undefined): Promise<Task | null> {
    if (!references) {
      return null
    }
    const refs = references.split(/\s+/)
    for (const ref of refs) {
      const task = await this.findTaskByMessageId(ref, 'References')
      if (task) {
        return task
      }
    }
    return null
  }

  /**
   * Find a task by short ID in subject line.
   */
  private async findTaskBySubjectShortId(subject: string | undefined): Promise<Task | null> {
    if (!subject) {
      return null
    }
    const shortId = extractShortIdFromSubject(subject)
    if (!shortId) {
      return null
    }
    const task = await this.taskRepository.getByShortId(shortId)
    if (task) {
      logger.debug({ shortId }, 'Found task via subject short ID')
      return task as Task
    }
    return null
  }

  /**
   * Send an error reply to the sender.
   * Uses the original recipient address as the from address.
   */
  private async sendErrorReply(
    email: InboundEmail,
    senderEmail: string,
    errorMessage: string,
  ): Promise<void> {
    try {
      // Extract the clean email address from the "to" field
      const fromAddress = extractEmailAddress(email.to)
      await this.emailOutbound.sendErrorReply(
        senderEmail,
        fromAddress,
        email.subject,
        errorMessage,
        email.messageId,
      )
    } catch (error) {
      logger.error({ err: error, senderEmail }, 'Failed to send error reply')
    }
  }

  /**
   * Handle a pending tool response from an email reply.
   * Extracted to reduce cognitive complexity.
   */
  private async handlePendingToolResponse(
    existingTask: Task,
    message: string,
    pendingTool: PendingToolInfo,
  ): Promise<EmailProcessingResult> {
    const result = await this.submitUserInputResponse(existingTask.taskId, message, pendingTool)

    // Update thread timestamp
    const thread = await this.threadTracker.findThreadByTaskId(existingTask.taskId, 'email')
    if (thread?.id) {
      await this.threadTracker.updateLastMessageAt(thread.id)
    }

    if (!result.success) {
      const errorMessage = result.error || 'Failed to submit response'
      logger.error(
        { taskId: existingTask.taskId, error: errorMessage },
        'Failed to submit user input response from email',
      )
      return {
        success: false,
        taskId: existingTask.taskId,
        isNewTask: false,
        error: errorMessage,
      }
    }

    logger.info(
      { taskId: existingTask.taskId, toolName: pendingTool.toolName },
      'User input response submitted from email',
    )

    return {
      success: true,
      taskId: existingTask.taskId,
      isNewTask: false,
    }
  }

  /**
   * Submit a user input response for an interactive tool (AskUserQuestion or ExitPlanMode).
   * Parses the user's email message and routes it to the sandbox.
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
      const snapshot = await this.workspaceRuntimeManager.getSnapshot(taskId)
      if (!snapshot) {
        return { success: false, error: 'Workspace not found' }
      }

      const sandboxUrl = getSandboxUrl(snapshot.executionTarget)
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
   * Parse a user's email message into the appropriate response format for the pending tool.
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
