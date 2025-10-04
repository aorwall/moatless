import {
  CommandExecutionSchema,
  FileContentSchema,
  FileQuerySchema,
  ServerLogsQuerySchema,
  TaskFileSearchSchema,
  WorkspaceRefreshConfigRequestSchema,
  WorkspaceSetupRequestSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import type { AppBindings } from '../app.js'
import { getTaskId, handleServiceError } from '../utils/error-handler.js'
import {
  extractFilePathFromWildcard,
  validateQueryParams,
  validateRequestBody,
} from '../utils/route-helpers.js'
import { readTaskLog } from '../utils/task-log.js'

export async function registerWorkspaceRoutes(app: Hono<AppBindings>, prefix: string) {
  // Get workspace status
  app.get(`${prefix}/:taskId/workspace-status`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    // Task validation now handled in workspace service
    const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)
    return c.json(workspaceStatus)
  })

  // List files in workspace
  app.get(`${prefix}/:taskId/files`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    const validatedQuery = validateQueryParams(c, FileQuerySchema)
    const { path = '/', recursive = false } = validatedQuery

    try {
      // Get workspace instance and call method directly
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const fileList = await workspace.listFiles(path, recursive)

      // Transform string array to match FileListResponseSchema
      const files = fileList.map((filePath) => ({
        name: filePath.split('/').pop() || filePath,
        path: filePath,
        type: 'file' as const, // For now, assume all are files
      }))

      return c.json({ files, totalCount: files.length })
    } catch (error) {
      handleServiceError(error, 'List files')
    }
  })

  // Read file content
  app.get(`${prefix}/:taskId/files/*`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)
    const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

    try {
      // Get workspace instance and call method directly
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const fileData = await workspace.readFileWithChanges(filePath)
      return c.json({
        content: fileData.content,
        encoding: 'utf-8',
        oldContent: fileData.oldContent,
        additions: fileData.additions,
        deletions: fileData.deletions,
        isModified: fileData.isModified,
      })
    } catch (error) {
      handleServiceError(error, 'Read file')
    }
  })

  // Write file content
  app.put(`${prefix}/:taskId/files/*`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)
    const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

    const validatedBody = await validateRequestBody(c, FileContentSchema)
    const { content } = validatedBody

    // Get workspace instance and call method directly
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    await workspace.writeFile(filePath, content)
    return c.json({ success: true })
  })

  // Search files by filename/path
  app.post(`${prefix}/:taskId/search-files`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    const body = await c.req.json()
    const validatedBody = TaskFileSearchSchema.parse(body)
    const { query, limit } = validatedBody

    try {
      // Get workspace instance and call method directly
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.searchFiles({
        query,
        limit,
      })
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Search files')
    }
  })

  // Get git diff for workspace
  app.get(`${prefix}/:taskId/git-diff`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    try {
      // Get workspace instance and call method directly
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const diffResult = await workspace.getGitDiff()
      return c.json({
        diff: diffResult.diffString,
        files: diffResult.files.map((f) => f.filename),
        hasChanges: diffResult.files.length > 0,
        totalChanges: diffResult.totalChanges,
        totalAdditions: diffResult.totalAdditions,
        totalDeletions: diffResult.totalDeletions,
        compareBase: diffResult.compareBase,
      })
    } catch (error) {
      handleServiceError(error, 'Get git diff')
    }
  })

  // Get git sync status with origin for workspace
  app.get(`${prefix}/:taskId/git-sync-status`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    try {
      // Get workspace status which now includes unified git status
      const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)

      if (workspaceStatus.gitStatus) {
        return c.json(workspaceStatus.gitStatus)
      }
      // If git status is not available, return a default response
      return c.json({
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
      })
    } catch (error) {
      handleServiceError(error, 'Get workspace status')
    }
  })

  // Get file tree with git changes
  app.get(`${prefix}/:taskId/file-tree-with-changes`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)
    const rootPath = c.req.query('path') || '.'

    try {
      // Get workspace instance and call method directly
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.getFileTreeWithChanges(rootPath)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Get file tree with changes')
    }
  })

  // Sync workspace with main branch
  app.post(`${prefix}/:taskId/sync-with-main`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    const baseBranch = c.req.query('baseBranch')

    try {
      // Get workspace instance and call method directly
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.syncWithMain(baseBranch)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Sync with main')
    }
  })

  // Sync workspace with origin (fetch from remote)
  app.post(`${prefix}/:taskId/sync-origin`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    const baseBranch = c.req.query('baseBranch')

    try {
      // Sync with origin and get updated status
      await services.workspace.syncWithOrigin(taskId, baseBranch)

      // Return updated workspace status
      const status = await services.workspace.getWorkspaceStatus(taskId)
      return c.json(status)
    } catch (error) {
      handleServiceError(error, 'Sync with origin')
    }
  })

  // Execute command in workspace
  app.post(`${prefix}/:taskId/execute`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    const body = await c.req.json()
    const validatedBody = CommandExecutionSchema.parse(body)
    const { command, timeout } = validatedBody

    // Get workspace instance and call method directly
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    const result = await workspace.runCommand(command, timeout)
    return c.json(result)
  })

  // Setup workspace
  app.post(`${prefix}/:taskId/setup`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    try {
      const body = await c.req.json().catch(() => ({}))
      WorkspaceSetupRequestSchema.parse(body)

      await services.workspace.setupWorkspace(taskId)

      return c.json({
        success: true,
        message: 'Workspace setup completed successfully',
        status: 'idle',
      })
    } catch (error) {
      handleServiceError(error, 'Setup workspace')
    }
  })

  // Refresh workspace configuration
  app.post(`${prefix}/:taskId/refresh-config`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    try {
      const body = await c.req.json().catch(() => ({}))
      const { restartServer = false } = WorkspaceRefreshConfigRequestSchema.parse(body)

      // Refresh configuration and get updated status
      const status = await services.workspace.refreshWorkspaceConfiguration(taskId, restartServer)

      return c.json({
        success: true,
        message: 'Configuration refreshed successfully',
        status: status.environmentStatus,
        serverRestarted: restartServer && status.capabilities.serverManagement,
      })
    } catch (error) {
      handleServiceError(error, 'Refresh workspace configuration')
    }
  })

  // Get workspace logs (all shell commands including setup and server)
  app.get(`${prefix}/:taskId/workspace/logs`, async (c) => {
    const services = c.get('services')
    const taskId = getTaskId(c)

    // Validate task exists
    const task = await services.storage.getTask(taskId)
    if (!task) {
      return c.json({ error: 'Task not found' }, 404)
    }

    // Parse query parameters
    const queryParams = c.req.query()
    const validatedQuery = ServerLogsQuerySchema.parse({
      lines: queryParams.lines ? Number.parseInt(queryParams.lines) : undefined,
    })
    const { lines } = validatedQuery

    // Read workspace logs from unified log file
    const logs = await readTaskLog(taskId, lines)
    return c.json({
      logs,
      lines: logs.split('\n').filter((line) => line.trim() !== '').length,
      truncated: lines !== undefined && lines > 0,
    })
  })
}
