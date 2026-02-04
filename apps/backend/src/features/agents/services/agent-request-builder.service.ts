import type { AgentRunRequest, FileAttachment } from '@moatless/sandbox-schemas'
import type { MessageContext, PreviewUrlContextData, UserContent } from '@moatless/types'
import { generateComponentList } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Logger } from 'pino'
import { logger } from '../../../utils/logger'
import { FigmaService } from '../../figma'
import { FileStorageService } from '../../file-storage'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { getWorkspacePathFromTarget } from '../../workspace/types/workspace-runtime.types'
import { getServerManagementPromptText } from '../prompts/preview-server-prompt'
import { getTaskMetadataPromptText } from '../prompts/task-metadata-prompt'

/**
 * Options for building an agent request.
 */
export interface AgentRequestBuildOptions {
  resumeSessionId?: string | undefined
  fileIds?: string[] | undefined
  contexts?: MessageContext[] | undefined
  externalPreviewUrl?: string | undefined
  taskName?: string | undefined
}

/**
 * AgentRequestBuilder - Builds AgentRunRequest objects for queue publishing
 *
 * Responsibilities:
 * - Build complete AgentRunRequest from task/iteration data
 * - Prepare messages with file attachments
 * - Generate system prompts
 * - Process contexts (Figma, preview URL, etc.)
 *
 * This is a pure request builder with no execution logic.
 * Agent execution is handled by the queue consumer in the sandbox.
 */
@injectable()
export class AgentRequestBuilder {
  constructor(
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(FileStorageService) private readonly uploadService: FileStorageService,
    @inject(FigmaService) private readonly figmaService: FigmaService,
  ) {}

  /**
   * Build a complete AgentRunRequest ready for queue publishing.
   *
   * @param taskId - Task ID
   * @param message - User message
   * @param turnNumber - Turn number for this request
   * @param agentMode - Agent mode (plan/edit)
   * @param model - Model to use
   * @param repositoryId - Repository ID
   * @param options - Build options (resumeSessionId, fileIds, contexts)
   */
  async buildRequest(
    taskId: string,
    message: string,
    turnNumber: number,
    agentMode?: 'edit' | 'plan',
    model?: string,
    _repositoryId?: string,
    options?: AgentRequestBuildOptions,
  ): Promise<AgentRunRequest> {
    const buildLogger = logger.child({ taskId, component: 'AgentRequestBuilder' })

    // Prepare message with files and contexts
    const {
      message: preparedMessage,
      contentBlocks,
      fileAttachments,
    } = await this.prepareMessageWithAttachments(
      taskId,
      message,
      options?.fileIds,
      options?.contexts,
      buildLogger,
    )

    // Build request options (per-request only, static config in REPO_CONFIG)
    const requestOptions = this.buildRequestOptions(
      taskId,
      turnNumber,
      agentMode,
      model,
      options?.resumeSessionId,
      buildLogger,
      options?.externalPreviewUrl,
      options?.taskName,
    )

    // Build content array: text message + any content blocks (images, etc.)
    const content: UserContent[] = [{ type: 'text', text: preparedMessage }]
    if (contentBlocks && contentBlocks.length > 0) {
      content.push(...contentBlocks)
    }

    // Build complete AgentRunRequest
    const agentRequest: AgentRunRequest = {
      content,
      ...requestOptions,
      fileAttachments,
    }

    buildLogger.debug(
      {
        contentLength: content.length,
        hasFileAttachments: (fileAttachments?.length ?? 0) > 0,
        hasResume: !!options?.resumeSessionId,
      },
      'Built agent request for queue',
    )

    return agentRequest
  }

  /**
   * Build request options with per-request configuration.
   *
   * Note: Static configuration (plugins, systemPrompt, timeout) is now in REPO_CONFIG.agentConfig
   * and read by the sandbox at startup. This method only includes per-request options.
   */
  private async buildRequestOptions(
    _taskId: string,
    _turnNumber: number,
    agentMode?: 'edit' | 'plan',
    model?: string,
    resumeSessionId?: string,
    buildLogger?: Logger,
    externalPreviewUrl?: string,
    taskName?: string,
  ): Promise<Omit<AgentRunRequest, 'content'>> {
    // Collect all prompt sections
    const promptSections: string[] = []

    // Add server management prompt if servers are configured
    const serverPrompt = await this.buildServerPrompt(_taskId, externalPreviewUrl, buildLogger)
    if (serverPrompt) {
      promptSections.push(serverPrompt)
    }

    // Add task metadata prompt if task has no custom name
    if (!taskName) {
      promptSections.push(getTaskMetadataPromptText())
      buildLogger?.debug('Task metadata prompt included (no custom name)')
    }

    // Combine prompts
    const appendSystemPrompt = promptSections.length > 0 ? promptSections.join('\n\n') : undefined

    if (appendSystemPrompt) {
      buildLogger?.debug(
        { promptLength: appendSystemPrompt.length },
        'Combined system prompt generated for request',
      )
    }

    // Only include per-request options
    // Static config (plugins, systemPrompt, timeout) is now in REPO_CONFIG.agentConfig
    const requestOptions: Omit<AgentRunRequest, 'content'> = {
      model,
      resume: resumeSessionId,
      agentMode,
      appendSystemPrompt,
    }

    return requestOptions
  }

  /**
   * Build server management prompt from workspace snapshot servers config.
   */
  private async buildServerPrompt(
    taskId: string,
    externalPreviewUrl?: string,
    buildLogger?: Logger,
  ): Promise<string | null> {
    try {
      const servers = await this.getServersFromRepoConfig(taskId)
      if (!servers || servers.length === 0) {
        // No servers configured — skip prompt
        if (externalPreviewUrl) {
          // Legacy fallback: still include prompt if external URL exists
          const prompt = getServerManagementPromptText(
            [{ name: 'preview', label: 'Preview' }],
            externalPreviewUrl,
          )
          buildLogger?.debug({ externalPreviewUrl }, 'Server management prompt included (legacy)')
          return prompt
        }
        return null
      }

      const prompt = getServerManagementPromptText(servers, externalPreviewUrl)
      buildLogger?.debug(
        { serverCount: servers.length, serverNames: servers.map((s) => s.name) },
        'Server management prompt included',
      )
      return prompt
    } catch (error) {
      buildLogger?.warn(
        { taskId, error: error instanceof Error ? error.message : String(error) },
        'Failed to build server prompt, skipping',
      )
      return null
    }
  }

  /**
   * Derive server list from repository config via task lookup.
   * Ports are assigned in the DB by autoPopulateServers, so we read them directly.
   */
  private async getServersFromRepoConfig(
    taskId: string,
  ): Promise<Array<{ name: string; label?: string; port: number }> | null> {
    const { appContainer } = await import('../../../container/container')
    const { TaskRepository } = await import('../../tasks/db/task.repository')
    const taskRepo = appContainer.get(TaskRepository)
    const task = await taskRepo.get(taskId)
    if (!task?.repositoryId) {
      return null
    }

    const { RepositoryService } = await import('../../repositories/services/repository.service')
    const repoService = appContainer.get(RepositoryService)
    const repo = await repoService.getRepositoryById(task.repositoryId)
    if (!repo?.servers || repo.servers.length === 0) {
      return null
    }

    return repo.servers.map((s) => {
      const entry: { name: string; label?: string; port: number } = { name: s.name, port: s.port! }
      if (s.label) {
        entry.label = s.label
      }
      return entry
    })
  }

  /**
   * Prepare message by loading all file attachments and appending contexts.
   */
  private async prepareMessageWithAttachments(
    taskId: string,
    message: string,
    fileIds: string[] | undefined,
    contexts: MessageContext[] | undefined,
    buildLogger: Logger,
  ): Promise<{
    message: string
    contentBlocks: UserContent[]
    fileAttachments: FileAttachment[]
  }> {
    // Load all uploaded files and get file data for K8s transfer
    const {
      contentBlocks: uploadedContentBlocks,
      fileInfoList,
      fileAttachments,
    } = await this.loadAllFileAttachments(fileIds, buildLogger)

    // Prepend file list to message if files are attached
    let messageWithFileInfo = message
    if (fileInfoList.length > 0) {
      const fileListHeader = 'Attached Files (you can read these files from the paths below):'
      const fileListItems = fileInfoList
        .map((file) => `- ${file.filename} (${file.mimeType})\n  Path: ${file.filePath}`)
        .join('\n')
      const fileListSection = `${fileListHeader}\n${fileListItems}`

      // Prepend file list to message
      messageWithFileInfo = message ? `${fileListSection}\n\n${message}` : fileListSection

      buildLogger.debug(
        {
          fileCount: fileInfoList.length,
          files: fileInfoList.map((f) => ({ filename: f.filename, path: f.filePath })),
        },
        'File list prepended to message',
      )
    }

    // Append contexts to message and collect Figma image content blocks
    const { message: preparedMessage, contentBlocks: contextContentBlocks } =
      await this.appendContextsToMessage(taskId, messageWithFileInfo, contexts, buildLogger)

    // Merge uploaded file content blocks with Figma context content blocks
    const allContentBlocks = [...uploadedContentBlocks, ...contextContentBlocks]

    buildLogger.debug(
      {
        uploadedContentBlockCount: uploadedContentBlocks.length,
        contextContentBlockCount: contextContentBlocks.length,
        totalContentBlockCount: allContentBlocks.length,
        hasFileList: fileInfoList.length > 0,
        fileAttachmentCount: fileAttachments.length,
      },
      'Message prepared with all content blocks and file attachments',
    )

    return { message: preparedMessage, contentBlocks: allContentBlocks, fileAttachments }
  }

  /**
   * Load all file attachments and collect metadata for message formatting.
   *
   * Files are now accessible via volume mounts at /mnt/uploads/{fileId}.
   * No need to transfer base64 data - agents read files directly from the mount.
   */
  private async loadAllFileAttachments(
    fileIds: string[] | undefined,
    buildLogger: Logger,
  ): Promise<{
    contentBlocks: UserContent[]
    fileInfoList: Array<{ filename: string; mimeType: string; filePath: string }>
    fileAttachments: FileAttachment[]
  }> {
    if (!fileIds || fileIds.length === 0) {
      return { contentBlocks: [], fileInfoList: [], fileAttachments: [] }
    }

    const fileInfoList: Array<{ filename: string; mimeType: string; filePath: string }> = []
    const fileAttachments: FileAttachment[] = []

    for (const fileId of fileIds) {
      try {
        // Get file metadata only - no need to load content since files are mounted
        const metadata = await this.uploadService.getFileMetadata(fileId)
        const filePath = this.uploadService.getFilePath(fileId)

        // Collect file info for message formatting
        fileInfoList.push({
          filename: metadata.filename,
          mimeType: metadata.mimeType,
          filePath,
        })

        // File metadata for the request (no base64 data - files are mounted)
        fileAttachments.push({
          fileId: metadata.fileId,
          filename: metadata.filename,
          mimeType: metadata.mimeType,
          // data field omitted - files accessible via mount at filePath
        })

        buildLogger.debug(
          {
            fileId,
            filename: metadata.filename,
            mimeType: metadata.mimeType,
            filePath,
          },
          'File attachment loaded (mounted at sandbox path)',
        )
      } catch (error) {
        buildLogger.error(
          {
            fileId,
            error: error instanceof Error ? error.message : String(error),
          },
          'Failed to get file attachment',
        )
        throw error
      }
    }

    buildLogger.info(
      {
        totalFiles: fileIds.length,
        totalAttachments: fileAttachments.length,
      },
      'All file attachments loaded',
    )

    // Return empty content blocks - files are only listed in the message
    return { contentBlocks: [], fileInfoList, fileAttachments }
  }

  /**
   * Append context items to message for agent reference.
   */
  private async appendContextsToMessage(
    taskId: string,
    message: string,
    contexts: MessageContext[] | undefined,
    buildLogger: Logger,
  ): Promise<{ message: string; contentBlocks: UserContent[] }> {
    if (!contexts || contexts.length === 0) {
      return { message, contentBlocks: [] }
    }

    const contextParts: string[] = []
    const contentBlocks: UserContent[] = []

    for (const context of contexts) {
      const result = await this.processContextItem(taskId, context, buildLogger)
      if (result.text) {
        contextParts.push(result.text)
      }
      if (result.imageBlock) {
        contentBlocks.push(result.imageBlock)
      }
    }

    const finalMessage = this.buildFinalMessage(message, contextParts)
    return { message: finalMessage, contentBlocks }
  }

  /**
   * Process a single context item and return text and optional image block.
   */
  private async processContextItem(
    taskId: string,
    context: MessageContext,
    buildLogger: Logger,
  ): Promise<{
    text: string | null
    imageBlock: import('@moatless/types').ImageContentBlock | null
  }> {
    if (context.type === 'element') {
      const componentList = generateComponentList(context.data)
      return { text: componentList, imageBlock: null }
    }

    if (context.type === 'figma') {
      const { contextText, imageBlock } = await this.processFigmaContext(
        taskId,
        context.data,
        buildLogger,
      )
      return { text: contextText, imageBlock }
    }

    if (context.type === 'previewUrl') {
      const previewUrlPart = this.processPreviewUrlContext(context.data, buildLogger)
      return { text: previewUrlPart, imageBlock: null }
    }

    return { text: null, imageBlock: null }
  }

  /**
   * Build final message with context parts.
   */
  private buildFinalMessage(message: string, contextParts: string[]): string {
    if (contextParts.length === 0) {
      return message
    }

    const contextSection = `Context:\n${contextParts.join('\n\n')}`
    return message ? `${message}\n\n${contextSection}` : contextSection
  }

  /**
   * Process Figma context and return formatted context text and optional image content block.
   */
  private async processFigmaContext(
    taskId: string,
    figmaData: { importId: string; fileName: string; nodeName: string },
    buildLogger: Logger,
  ): Promise<{
    contextText: string | null
    imageBlock: import('@moatless/types').ImageContentBlock | null
  }> {
    try {
      // Get workspace path from coordinator
      const sandboxRecord = await this.runtimeManager.getSandboxRecord(taskId)
      if (!sandboxRecord) {
        throw new Error('Cannot process Figma context - workspace not found')
      }

      const workspacePath = getWorkspacePathFromTarget(sandboxRecord.executionTarget)

      // Use FigmaService to get context for workspace (returns both text and optional image block)
      const { contextText, imageBlock } = await this.figmaService.getFigmaContextForWorkspace(
        figmaData.importId,
        workspacePath,
      )

      buildLogger.debug(
        {
          importId: figmaData.importId,
          hasImageBlock: imageBlock !== null,
        },
        'Figma context processed successfully',
      )

      return { contextText, imageBlock }
    } catch (error) {
      buildLogger.warn(
        {
          importId: figmaData.importId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to process Figma context',
      )
      return { contextText: null, imageBlock: null }
    }
  }

  /**
   * Process Preview URL context and return formatted context.
   */
  private processPreviewUrlContext(
    previewUrlData: PreviewUrlContextData,
    buildLogger: Logger,
  ): string | null {
    // Guard: Ignore if path is not set
    if (!previewUrlData.path) {
      return null
    }

    try {
      const displayUrl = previewUrlData.url || previewUrlData.path

      buildLogger.debug(
        {
          path: previewUrlData.path,
          displayUrl,
        },
        'Preview URL context processed successfully',
      )

      return `User is currently viewing: ${displayUrl}`
    } catch (error) {
      buildLogger.warn(
        {
          path: previewUrlData.path,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to process Preview URL context',
      )
      return null
    }
  }
}
