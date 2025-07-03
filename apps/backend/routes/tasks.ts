import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import { TaskNotFoundError, WorkspaceError } from '@moatless/shared/constants/errors'
import {
  CreateTaskRequestSchema,
  type EventListResponse,
  SendMessageRequestSchema,
  TaskListQuerySchema,
  type TaskListResponse,
} from '@moatless/shared/types/api'
import type { EventType } from '@moatless/shared/types/event'

// Schema for query params
const EventTypeQuerySchema = z.object({
  type: z.string().optional(),
})

const FileQuerySchema = z.object({
  path: z.string().optional().default('/'),
  recursive: z.boolean().optional().default(false),
})

const FileContentSchema = z.object({
  content: z.string(),
})

const FileSearchSchema = z.object({
  query: z.string(),
  limit: z.number().optional(),
})

const CommandExecutionSchema = z.object({
  command: z.string(),
  timeout: z.number().optional(),
})

const ServerLogsQuerySchema = z.object({
  lines: z.number().optional(),
})

const MergeTaskSchema = z.object({
  targetBranch: z.string().optional().default('main'),
  commitMessage: z.string().optional(),
})


export async function registerTaskRoutes(app: Hono<AppBindings>, prefix: string) {

  // Create a new task
  app.post(prefix, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    
    const body = await c.req.json()
    const validatedBody = CreateTaskRequestSchema.parse(body)
    const { model, filePaths, ...taskData } = validatedBody
    
    const taskId = await taskService.createTask({ ...taskData, model, filePaths })

    // Get the task to return the current status
    const task = await taskService.getTask(taskId)
    return c.json({ taskId, status: task?.status || 'open' })
  })

  // Get task by ID
  app.get(`${prefix}/:taskId`, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    const task = await taskService.getTask(taskId)

    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    return c.json(task)
  })

  // List all tasks with pagination
  app.get(prefix, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    
    const queryParams = c.req.query()
    const validatedQuery = TaskListQuerySchema.parse(queryParams)
    const { limit, offset, status } = validatedQuery
    
    const tasks = await taskService.listTasks(limit, offset, status)

    const response: TaskListResponse = {
      tasks: tasks,
      items: tasks,
      pagination: {
        limit,
        offset,
        total: tasks.length, // Note: This is simplified, real implementation would need total count
      },
    }
    return c.json(response)
  })

  // Send message to task
  app.post(`${prefix}/:taskId/messages`, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    const body = await c.req.json()
    const validatedBody = SendMessageRequestSchema.parse(body)
    const { message, resume, model, fileIds, filePaths, commandName } = validatedBody

    await taskService.sendMessage(taskId, message, resume, model, fileIds, filePaths, commandName)
    return c.json({ status: 'sent', timestamp: new Date().toISOString() })
  })

  // Get task events without pagination
  app.get(`${prefix}/:taskId/events`, async (c) => {
    const services = c.get('services')
    const { task: taskService, event: eventService } = services
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    const queryParams = c.req.query()
    const validatedQuery = EventTypeQuerySchema.parse(queryParams)
    const { type } = validatedQuery

    // Verify task exists
    const task = await taskService.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    const events = await eventService.getEvents(
      taskId,
      undefined, // no limit
      0, // offset 0
      type as EventType | undefined,
      task,
    )

    const response: EventListResponse = {
      events,
      items: events,
    }
    return c.json(response)
  })

  // Delete task
  app.delete(`${prefix}/:taskId`, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    await taskService.deleteTask(taskId)
    return c.json({ status: 'deleted' })
  })

  // Get workspace status
  app.get(`${prefix}/:taskId/workspace-status`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation now handled in workspace service
    const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)
    return c.json(workspaceStatus as any)
  })

  // Stop agent
  app.post(`${prefix}/:taskId/stop`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Stop agent through AgentService
    const stopped = await services.agent.stopAgent(taskId)

    return c.json({
      status: stopped ? 'stopped' : 'not_running',
      taskId,
    })
  })

  // Close task (teardown workspace, stop server, set status to closed)
  app.post(`${prefix}/:taskId/close`, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    await taskService.closeTask(taskId)

    return c.json({
      status: 'closed',
      taskId,
    })
  })

  // List files in workspace
  app.get(`${prefix}/:taskId/files`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    const queryParams = c.req.query()
    const validatedQuery = FileQuerySchema.parse(queryParams)
    const { path = '/', recursive = false } = validatedQuery

    try {
      // Task validation and capability check now handled in workspace service
      const files = await services.workspace.listFiles(taskId, path, recursive)
      return c.json({ files, totalCount: files.length })
    } catch (error) {
      if (error instanceof TaskNotFoundError) {
        throw new HTTPException(404, { message: 'Task not found' })
      }
      if (error instanceof Error && error.message.includes('Path not found')) {
        throw new HTTPException(404, { message: error.message })
      }
      if (error instanceof Error && error.message.includes('Access denied')) {
        throw new HTTPException(403, { message: error.message })
      }
      if (error instanceof Error && error.message.includes('Cannot list files')) {
        throw new HTTPException(403, { message: error.message })
      }
      throw error
    }
  })

  // Read file content
  app.get(`${prefix}/:taskId/files/*`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    // Get the wildcard portion of the path
    const path = c.req.path
    const basePattern = `${prefix}/${taskId}/files/`
    const filePath = path.replace(basePattern, '')

    if (!filePath) {
      throw new HTTPException(400, { message: 'File path is required' })
    }

    try {
      // Task validation and capability check now handled in workspace service
      const content = await services.workspace.readFile(taskId, filePath)
      return c.json({ content, encoding: 'utf-8' })
    } catch (error) {
      if (error instanceof TaskNotFoundError) {
        throw new HTTPException(404, { message: 'Task not found' })
      }
      if (error instanceof Error && error.message.includes('File not found')) {
        throw new HTTPException(404, { message: error.message })
      }
      if (error instanceof Error && error.message.includes('Access denied')) {
        throw new HTTPException(403, { message: error.message })
      }
      if (error instanceof Error && error.message.includes('File too large')) {
        throw new HTTPException(413, { message: error.message })
      }
      if (error instanceof Error && error.message.includes('Path is not a file')) {
        throw new HTTPException(400, { message: error.message })
      }
      if (error instanceof Error && error.message.includes('Cannot read file')) {
        throw new HTTPException(403, { message: error.message })
      }
      throw error
    }
  })

  // Write file content
  app.put(`${prefix}/:taskId/files/*`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    // Get the wildcard portion of the path
    const path = c.req.path
    const basePattern = `${prefix}/${taskId}/files/`
    const filePath = path.replace(basePattern, '')
    
    if (!filePath) {
      throw new HTTPException(400, { message: 'File path is required' })
    }
    
    const body = await c.req.json()
    const validatedBody = FileContentSchema.parse(body)
    const { content } = validatedBody

    // Task validation and capability check now handled in workspace service
    await services.workspace.writeFile(taskId, filePath, content)
    return c.json({ success: true })
  })

  // Search files by filename/path
  app.post(`${prefix}/:taskId/search-files`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    const body = await c.req.json()
    const validatedBody = FileSearchSchema.parse(body)
    const { query, limit } = validatedBody

    try {
      // Task validation and capability check now handled in workspace service
      const result = await services.workspace.searchFiles(taskId, {
        query,
        limit,
      })
      return c.json(result)
    } catch (error) {
      if (error instanceof TaskNotFoundError) {
        throw new HTTPException(404, { message: 'Task not found' })
      }
      if (error instanceof Error && error.message.includes('Cannot search files')) {
        throw new HTTPException(403, { message: error.message })
      }
      throw error
    }
  })

  // Get git diff for workspace
  app.get(`${prefix}/:taskId/git-diff`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    try {
      // Task validation and capability check now handled in workspace service
      const diff = await services.workspace.getGitDiff(taskId)
      return c.json({
        diff,
        files: [], // TODO: Parse diff to extract file list
        hasChanges: (diff as string).length > 0,
      })
    } catch (error) {
      if (error instanceof TaskNotFoundError) {
        throw new HTTPException(404, { message: 'Task not found' })
      }
      if (error instanceof WorkspaceError) {
        throw new HTTPException(error.statusCode as any, { message: error.message })
      }
      if (error instanceof Error && error.message.includes('Cannot access git')) {
        throw new HTTPException(403, { message: error.message })
      }
      throw error
    }
  })

  // Execute command in workspace
  app.post(`${prefix}/:taskId/execute`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    const body = await c.req.json()
    const validatedBody = CommandExecutionSchema.parse(body)
    const { command, timeout } = validatedBody

    // Task validation and capability check now handled in workspace service
    const result = await services.workspace.runCommand(taskId, command, timeout)
    return c.json(result)
  })

  // Start development server
  app.post(`${prefix}/:taskId/server/start`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation and capability check now handled in workspace service
    const result = await services.workspace.startServer(taskId)
    return c.json(result as any)
  })

  // Stop development server
  app.post(`${prefix}/:taskId/server/stop`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation and capability check now handled in workspace service
    const stopped = await services.workspace.stopServer(taskId)
    return c.json({
      stopped,
      runtime: 0, // TODO: Calculate actual runtime
    })
  })

  // Restart development server
  app.post(`${prefix}/:taskId/server/restart`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }

    // Task validation and capability check now handled in workspace service
    const result = await services.workspace.restartServer(taskId)
    return c.json(result as any)
  })

  // Get server status
  app.get(`${prefix}/:taskId/server/status`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    // Task validation and capability check now handled in workspace service
    const status = await services.workspace.getServerStatus(taskId)
    return c.json(status as any)
  })

  // Get server logs
  app.get(`${prefix}/:taskId/server/logs`, async (c) => {
    const services = c.get('services')
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    const queryParams = c.req.query()
    const validatedQuery = ServerLogsQuerySchema.parse(queryParams)
    const { lines } = validatedQuery

    // Task validation and capability check now handled in workspace service
    const logs = await services.workspace.getServerLogs(taskId, lines)
    return c.json({
      logs,
      lines: logs.split('\n').length,
      truncated: false, // TODO: Implement truncation logic
    })
  })

  // Merge task changes
  app.post(`${prefix}/:taskId/merge`, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    
    const taskId = c.req.param('taskId')
    if (!taskId) {
      throw new HTTPException(400, { message: 'Task ID is required' })
    }
    
    let body: any = {}
    try {
      body = await c.req.json()
    } catch {
      // Body is optional for this endpoint
    }
    const validatedBody = MergeTaskSchema.parse(body || {})
    const { targetBranch = 'main', commitMessage } = validatedBody

    // Import git utils
    const { getCurrentBranch, hasUncommittedChanges, mergeGitBranch, checkForConflicts } =
      await import('../utils/git-utils.js')

    // Get task to verify it exists
    const task = await taskService.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Get workspace through workspace service
    const workspace = await services.workspace.getOrCreateWorkspace(taskId)
    if (!workspace) {
      throw new Error(`No workspace found for task ${taskId}`)
    }

    const workspacePath = workspace.getWorkspacePath()

    // Get current branch
    const currentBranch = await getCurrentBranch(workspacePath)

    // Extract main repo path from worktree for additional checks
    const gitPath = `${workspacePath}/.git`
    let mainRepoPath = workspacePath

    try {
      const { exec } = await import('node:child_process')
      const { promisify } = await import('node:util')
      const execAsync = promisify(exec)

      const { stdout: gitFileContent } = await execAsync(`cat "${gitPath}"`, {
        cwd: workspacePath,
        timeout: 5000,
      })

      // If .git is a file (worktree), extract main repo path
      if (gitFileContent.startsWith('gitdir:')) {
        const gitDir = gitFileContent.split(':')[1].trim()
        // Extract main repo path from worktree git dir
        const pathParts = gitDir.split('/')
        const worktreesIndex = pathParts.lastIndexOf('worktrees')
        if (worktreesIndex > 0) {
          mainRepoPath = pathParts.slice(0, worktreesIndex - 1).join('/')
        }
      }
    } catch {
      // .git is a directory, we're in the main repo
    }

    // Check for uncommitted changes in the main repository target branch
    const mainHasChanges = await hasUncommittedChanges(mainRepoPath)
    if (mainHasChanges) {
      throw new HTTPException(400, { 
        message: `Cannot merge: target branch '${targetBranch}' has uncommitted changes in the main repository` 
      })
    }

    // Check for conflicts before attempting merge
    const hasConflicts = await checkForConflicts(workspacePath, targetBranch)
    if (hasConflicts) {
      throw new HTTPException(400, { 
        message: `Cannot merge: conflicts detected between '${currentBranch}' and '${targetBranch}'` 
      })
    }

    // Perform the merge
    const mergeCommit = await mergeGitBranch(
      workspacePath,
      currentBranch,
      targetBranch,
      commitMessage,
    )

    // Check if worktree has uncommitted changes after merge
    const worktreeHasChanges = await hasUncommittedChanges(workspacePath)

    // If no uncommitted changes on the worktree branch after merge, update task status to "merged"
    if (!worktreeHasChanges) {
      await taskService.updateTask(taskId, { status: 'merged' })
    }

    return c.json({
      success: true,
      mergeCommit,
      sourceBranch: currentBranch,
      targetBranch,
    })
  })

}
