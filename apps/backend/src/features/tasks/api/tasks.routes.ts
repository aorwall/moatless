import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  closeTaskRoute,
  createTaskRoute,
  createVibeTaskRoute,
  deleteTaskRoute,
  getCommandsRoute,
  getTaskRoute,
  listTasksRoute,
  sendCommandRoute,
  sendMessageRoute,
  startTaskRoute,
  stopAgentRoute,
  updateTaskMetadataRoute,
  updateTaskTagsRoute,
  updateTaskVisibilityRoute,
} from '@moatless/api-contract/tasks'
import type { TaskListResponse } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { AgentControlService } from '../../agents/services/agent-control.service'
import { loadProxyConfig, type ProxyEnvironmentConfig } from '../../preview/config/proxy-config'
import { PreviewOrchestratorService } from '../../preview/services/preview-orchestrator.service'
import { RepositoryService } from '../../repositories/services/repository.service'
import type { SandboxRecord } from '../../workspace/db/workspace-sandbox.schema'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskService } from '../services/task.service'

export function registerTaskRoutes(app: OpenAPIHono<AppBindings>) {
  // Create a new task
  app.openapi(createTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const body = c.req.valid('json')
    const { model, fileIds, ...taskData } = body

    try {
      const task = await taskService.createTask({
        ...taskData,
        model,
        fileIds,
      })
      return c.json({ taskId: task.taskId, status: task.status, task })
    } catch (error) {
      handleServiceError(error, 'Create task')
    }
  })

  // Create a new vibe task from template
  app.openapi(createVibeTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const body = c.req.valid('json')
    const { fileIds, ...taskData } = body

    try {
      const taskId = await taskService.createVibeTask({
        ...taskData,
        fileIds,
      })

      // Get the task to return the current status
      const task = await taskService.getTask(taskId)
      return c.json({ taskId, status: task?.status || 'open', task })
    } catch (error) {
      handleServiceError(error, 'Create vibe task')
    }
  })

  // Get task by ID
  app.openapi(getTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')

    try {
      const task = await taskService.getTask(taskId)
      return c.json(task)
    } catch (error) {
      handleServiceError(error, 'Get task')
    }
  })

  // List all tasks with pagination
  app.openapi(listTasksRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { limit, offset, status, tag, createdBy, repositoryId } = c.req.valid('query')

    const filters: { tag?: string; createdBy?: string; repositoryId?: string } = {}
    if (tag) {
      filters.tag = tag
    }
    if (createdBy) {
      filters.createdBy = createdBy
    }
    if (repositoryId) {
      filters.repositoryId = repositoryId
    }

    const { tasks, total } = await taskService.listTasks(limit, offset, status, filters)

    const response: TaskListResponse = {
      items: tasks,
      pagination: {
        limit,
        offset,
        total,
        hasMore: total !== undefined ? offset + limit < total : undefined,
      },
    }
    return c.json(response)
  })

  // Delete task (custom route that returns 200 with body, not 204 No Content)
  app.openapi(deleteTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')

    try {
      await taskService.deleteTask(taskId)
      return c.json({ status: 'deleted' })
    } catch (error) {
      handleServiceError(error, 'Delete task')
    }
  })

  // Update task visibility
  app.openapi(updateTaskVisibilityRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')
    const { visibility } = c.req.valid('json')

    try {
      const task = await taskService.getTask(taskId)
      task.visibility = visibility
      await taskService.updateTask(taskId, task)
      return c.json({ success: true as const })
    } catch (error) {
      handleServiceError(error, 'Update task visibility')
    }
  })

  // Update task metadata (name and description)
  app.openapi(updateTaskMetadataRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')
    const { name, description } = c.req.valid('json')

    try {
      // Build updates object with only defined values
      const updates: { name?: string; description?: string } = {}
      if (name !== undefined) {
        updates.name = name
      }
      if (description !== undefined) {
        updates.description = description
      }

      await taskService.updateTaskWithEvent(taskId, updates)
      return c.json({ success: true as const })
    } catch (error) {
      handleServiceError(error, 'Update task metadata')
    }
  })

  // Update task tags
  app.openapi(updateTaskTagsRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')
    const { tags } = c.req.valid('json')

    try {
      const resolvedTags = await taskService.setTaskTags(taskId, tags)
      return c.json({ tags: resolvedTags })
    } catch (error) {
      handleServiceError(error, 'Update task tags')
    }
  })

  // Send message to task (also handles user input responses for interactive tools)
  app.openapi(sendMessageRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')
    const {
      message,
      resume,
      model,
      fileIds,
      contexts,
      commandName,
      agentMode,
      toolUseId,
      toolName,
      toolResponse,
    } = c.req.valid('json')

    await taskService.sendMessage(
      taskId,
      message,
      resume,
      model,
      fileIds,
      commandName,
      agentMode,
      contexts,
      toolUseId,
      toolName,
      toolResponse,
    )
    return c.json({ status: 'sent', timestamp: new Date().toISOString() })
  })

  // Get available commands for task
  app.openapi(getCommandsRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')

    const commands = await taskService.getAvailableCommands(taskId)
    return c.json({ commands })
  })

  // Send command to task
  app.openapi(sendCommandRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')
    const { commandName, resume, model, fileIds } = c.req.valid('json')

    await taskService.sendCommand(taskId, commandName, resume, model, fileIds)
    return c.json({ status: 'sent', timestamp: new Date().toISOString() })
  })

  // Start task
  app.openapi(startTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')

    try {
      await taskService.startTask(taskId)
      return c.json({
        status: 'started',
        taskId,
      })
    } catch (error) {
      if (error instanceof TaskNotFoundError) {
        throw new HTTPException(404, { message: error.message })
      }
      if (error instanceof ValidationError) {
        throw new HTTPException(400, { message: error.message })
      }
      throw error
    }
  })

  // Stop agent
  app.openapi(stopAgentRoute, async (c) => {
    const agentControlService = appContainer.get(AgentControlService)
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const { taskId } = c.req.valid('param')

    try {
      // Check if agent is currently running
      const wasRunning = await runtimeManager.isAgentActuallyRunning(taskId)

      // Stop agent (no-op if not running)
      await agentControlService.stopAgent(taskId)

      return c.json({
        status: wasRunning ? 'stopped' : 'not_running',
        taskId,
      })
    } catch (error) {
      handleServiceError(error, 'Stop agent')
    }
  })

  // Close task (teardown workspace, stop server, set status to closed)
  app.openapi(closeTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')

    await taskService.closeTask(taskId)

    return c.json({
      status: 'closed',
      taskId,
    })
  })

  // Get available servers for a task (non-OpenAPI for now)
  app.get('/api/v1/tasks/:taskId/servers', async (c) => {
    const taskId = c.req.param('taskId')
    const servers = await buildServersList(taskId)
    return c.json({ servers })
  })
}

/**
 * Build servers list for a task with status, URL, and capabilities
 */
async function buildServersList(taskId: string): Promise<
  Array<{
    name: string
    label: string
    port: number
    status: string
    url: string | undefined
    error: string | undefined
    default: boolean
    capabilities: ServerCapabilities
  }>
> {
  const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
  const record = await runtimeManager.getSandboxRecord(taskId)
  if (!record) {
    return []
  }

  const serversMap = record.executionTarget?.servers ?? {}
  const proxyConfig = loadProxyConfig()

  // Get repositoryId from task record
  const taskService = appContainer.get(TaskService)
  let repositoryId: string | undefined
  try {
    const task = await taskService.getTask(taskId)
    repositoryId = task?.repositoryId
  } catch {
    // Best-effort
  }

  const repoDefaultMap = await fetchRepoDefaultMap(repositoryId)

  const entries = Object.entries(serversMap)
  const resolved = await Promise.all(
    entries.map(([name]) => resolveServerStatus(name, taskId, record, proxyConfig)),
  )

  return entries.map(([name, entry], i) => {
    const r = resolved[i] ?? { status: 'stopped', url: undefined, error: undefined }
    return {
      name,
      label: entry.label ?? name,
      port: entry.port,
      status: r.status,
      url: r.url,
      error: r.error,
      default: repoDefaultMap.get(name) ?? false,
      capabilities: getServerCapabilities(name),
    }
  })
}

/**
 * Fetch default server flags from repository config
 */
async function fetchRepoDefaultMap(
  repositoryId: string | undefined,
): Promise<Map<string, boolean>> {
  const repoDefaultMap = new Map<string, boolean>()
  if (!repositoryId) {
    return repoDefaultMap
  }

  try {
    const repoService = appContainer.get(RepositoryService)
    const repo = await repoService.getRepositoryById(repositoryId)
    if (repo?.servers) {
      for (const s of repo.servers) {
        if (s.default) {
          repoDefaultMap.set(s.name, true)
        }
      }
    }
  } catch {
    // Repo lookup is best-effort
  }

  return repoDefaultMap
}

/**
 * Resolve status, url, and error for a single server entry by querying the sandbox.
 */
async function resolveServerStatus(
  name: string,
  taskId: string,
  record: SandboxRecord,
  proxyConfig: ProxyEnvironmentConfig,
): Promise<{ status: string; url: string | undefined; error: string | undefined }> {
  // "files" server is a static proxy — its status is derived from environment readiness
  if (name === 'files') {
    const envReady = record.executionTarget?.mode !== undefined
    const status = envReady ? 'started' : 'stopped'
    const url =
      proxyConfig.useHostnameRouting && proxyConfig.proxyDomain
        ? `${proxyConfig.proxyProtocol}://${taskId}--${name}.${proxyConfig.proxyDomain}`
        : undefined
    return { status, url, error: undefined }
  }

  // For all other servers (preview, storybook, etc.), fetch live status from sandbox
  try {
    const orchestrator = appContainer.get(PreviewOrchestratorService)
    const serverStatus = await orchestrator.getServerStatus(taskId, name)

    // If the orchestrator returned a URL, use it; otherwise always generate a proxy URL
    // The proxy handles unreachable states with error pages + auto-refresh
    const url =
      serverStatus.url ||
      (proxyConfig.useHostnameRouting && proxyConfig.proxyDomain
        ? `${proxyConfig.proxyProtocol}://${taskId}${name === 'preview' ? '' : `--${name}`}.${proxyConfig.proxyDomain}`
        : undefined)

    return {
      status: serverStatus.status,
      url,
      error: serverStatus.error,
    }
  } catch {
    // Orchestrator not available — fall back to stopped with proxy URL
    const url =
      proxyConfig.useHostnameRouting && proxyConfig.proxyDomain
        ? `${proxyConfig.proxyProtocol}://${taskId}${name === 'preview' ? '' : `--${name}`}.${proxyConfig.proxyDomain}`
        : undefined
    return { status: 'stopped', url, error: undefined }
  }
}

interface ServerCapabilities {
  addressBar: boolean
  inspector: boolean
  reload: boolean
  openExternal: boolean
  viewport: boolean
  console: boolean
}

/**
 * Derive server capabilities from server name.
 * In the future this could be configurable per-server in ServerConfig.
 */
function getServerCapabilities(serverName: string): ServerCapabilities {
  switch (serverName) {
    case 'files':
      return {
        addressBar: false,
        inspector: false,
        reload: true,
        openExternal: true,
        viewport: false,
        console: false,
      }
    case 'preview':
      return {
        addressBar: true,
        inspector: true,
        reload: true,
        openExternal: true,
        viewport: true,
        console: true,
      }
    default:
      return {
        addressBar: true,
        inspector: false,
        reload: true,
        openExternal: true,
        viewport: false,
        console: false,
      }
  }
}
