import { serve } from '@hono/node-server'
import type { TaskIteration } from '@moatless/types/types/task'
import { Hono } from 'hono'
import { logger as loggerMiddleware } from './middleware/logger.js'
import { requestId } from './middleware/request-id.js'
import { AgentService, WorkspaceService } from './services/index.js'
import { PreviewServer } from './services/preview-server.js'
import { FileStorage } from './services/storage.js'
import { WorkerEventService } from './services/worker-event.service.js'
import { createLogger } from './utils/logger.js'
import { LocalWorkspace } from './workspaces/local.js'

export interface WorkerModeConfig {
  apiUrl: string
  apiKey?: string
  workspaceDir: string
  dataDir?: string // Optional data directory for storage (defaults to /data)
}

/**
 * Start the backend in worker mode
 *
 * Worker mode is used when running the backend inside a Docker container
 * to execute agent tasks. It uses:
 * - WorkerEventService to forward events to the main backend
 * - FileStorage with a mounted data volume for persistence
 * - LocalWorkspace for agent execution within the container
 */
export async function startWorkerMode(config: WorkerModeConfig): Promise<void> {
  const logger = createLogger()
  logger.info('Starting backend in worker mode', {
    apiUrl: config.apiUrl,
    workspaceDir: config.workspaceDir,
    dataDir: config.dataDir || '/data',
    logLevel: process.env.LOG_LEVEL || 'info',
  })

  // Initialize services for worker mode
  const taskId = process.env.WORKER_TASK_ID

  // WorkerEventService forwards events to main backend through task-specific endpoints
  const eventService = new WorkerEventService(config.apiUrl, config.apiKey, taskId)

  // Use regular FileStorage with mounted volume path
  // The data directory is mounted from the host, allowing types storage
  const dataDir = config.dataDir || '/data'
  const storage = new FileStorage(dataDir)

  // Create workspace service that only uses LocalWorkspace
  const workspaceService = new WorkspaceService(null as any, eventService as any, storage)

  // Override workspace factory to always use LocalWorkspace
  workspaceService.createWorkspace = async (taskId: string, repositoryConfig: any) => {
    return new LocalWorkspace(
      taskId,
      repositoryConfig,
      eventService as any,
      undefined,
      undefined,
      config.workspaceDir,
    )
  }

  // Create agent service
  const agentService = new AgentService(storage, eventService as any, workspaceService)

  // Create preview server instance for this worker
  let previewServer: PreviewServer | undefined

  // Create HTTP server with proper type bindings
  const app = new Hono<{
    Variables: {
      requestId: string
    }
  }>()

  // Request ID middleware (must be first)
  app.use('*', requestId)

  // Logger middleware
  app.use('*', loggerMiddleware)

  // Health check endpoint
  app.get('/health', (c) =>
    c.json({
      status: 'ok',
      mode: 'worker',
      workspaceDir: config.workspaceDir,
      dataDir: dataDir,
      taskId: taskId,
    }),
  )

  // Agent execution endpoint
  app.post('/execute-agent', async (c) => {
    const body = await c.req.json()
    const { taskId, message, iteration, agentType, resumeSessionId, model } = body

    logger.info('Received agent execution request', {
      taskId,
      agentType,
      iterationNumber: iteration.iterationNumber,
    })

    try {
      await agentService.executeAgent(
        taskId,
        message,
        iteration as TaskIteration,
        agentType || 'claude-code',
        resumeSessionId,
        model,
      )

      logger.info('Agent execution completed', { taskId })
      return c.json({ success: true })
    } catch (error) {
      logger.error('Agent execution failed', error as Error, { taskId })
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  // Stop agent endpoint
  app.post('/stop-agent', async (c) => {
    const body = await c.req.json()
    const { taskId } = body

    logger.info('Received agent stop request', { taskId })

    try {
      // Stop the agent
      const agentStopped = await agentService.stopAgent(taskId)

      // Also stop the workspace if it exists
      let workspaceStopped = false
      try {
        const workspace = await workspaceService.getWorkspace(taskId)
        if (workspace) {
          workspaceStopped = await workspace.stopAgent()
        }
      } catch (error) {
        // Workspace might not exist, which is okay
        logger.warn('Could not stop workspace agent', {
          error: error instanceof Error ? error.message : String(error),
        })
      }

      logger.info('Agent stop completed', { taskId, agentStopped, workspaceStopped })
      return c.json({
        success: true,
        agentStopped,
        workspaceStopped,
      })
    } catch (error) {
      logger.error('Agent stop failed', error as Error, { taskId })
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  // Preview server endpoints
  app.post('/server/start', async (c) => {
    const body = await c.req.json()
    const { serverCommand, envVars } = body

    try {
      // Get repository config from storage if available
      let repositoryConfig: any = null
      try {
        const repoData = await storage.getRepository('default')
        repositoryConfig = repoData
      } catch (error) {
        logger.warn('Could not load repository config, using defaults', {
          error: error instanceof Error ? error.message : String(error),
        })
      }

      // Use server command from request or fall back to repository config
      const finalServerCommand = serverCommand || repositoryConfig?.codeExecution?.serverCommand
      if (!finalServerCommand) {
        return c.json(
          {
            success: false,
            error: 'No server command configured',
          },
          400,
        )
      }

      // Create preview server instance
      previewServer = new PreviewServer(
        taskId || 'worker',
        config.workspaceDir,
        finalServerCommand,
        envVars || repositoryConfig?.codeExecution?.envVars || {},
      )

      const result = await previewServer.start()

      logger.info('Preview server started', {
        port: result.port,
        pid: result.pid,
      })

      return c.json({
        success: true,
        running: result.running,
        port: result.port,
        pid: result.pid,
        url: `http://localhost:${result.port}`,
        uptimeSeconds: result.uptime_seconds,
      })
    } catch (error) {
      logger.error('Failed to start preview server', error as Error)
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  app.post('/server/stop', async (c) => {
    try {
      if (!previewServer) {
        return c.json({ success: true, message: 'No server running' })
      }

      const stopped = await previewServer.stop()
      previewServer = undefined

      logger.info('Preview server stopped', { success: stopped })

      return c.json({
        success: stopped,
      })
    } catch (error) {
      logger.error('Failed to stop preview server', error as Error)
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  app.get('/server/status', async (c) => {
    try {
      if (!previewServer) {
        return c.json({
          running: false,
          configured: false,
          uptimeSeconds: 0,
        })
      }

      const status = previewServer.getStatus()

      return c.json(status)
    } catch (error) {
      logger.error('Failed to get server status', error as Error)
      return c.json(
        {
          running: false,
          configured: false,
          uptimeSeconds: 0,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  app.get('/server/logs', async (c) => {
    try {
      const lines = Number(c.req.query('lines')) || 100

      if (!previewServer) {
        return c.json({
          logs: 'No server running',
        })
      }

      const logs = await previewServer.getLogs(lines)

      return c.json({
        logs,
      })
    } catch (error) {
      logger.error('Failed to get server logs', error as Error)
      return c.json(
        {
          logs: `Error getting logs: ${error instanceof Error ? error.message : 'Unknown error'}`,
        },
        500,
      )
    }
  })

  // Command execution endpoint
  app.post('/command', async (c) => {
    const body = await c.req.json()
    const { command, timeout } = body

    try {
      // Get workspace to execute command
      const workspace = await workspaceService.getWorkspace(taskId!)
      if (!workspace) {
        return c.json(
          {
            exit_code: 1,
            stdout: '',
            stderr: 'No workspace available',
          },
          400,
        )
      }

      const result = await workspace.runCommand(command, timeout)

      return c.json({
        exit_code: result.exitCode || 0,
        stdout: result.output || '',
        stderr: '',
      })
    } catch (error) {
      logger.error('Command execution failed', error as Error)
      return c.json(
        {
          exit_code: 1,
          stdout: '',
          stderr: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  const port = Number.parseInt(process.env.PORT || '8001')
  logger.info(`Worker HTTP server starting on port ${port}`)

  serve({
    fetch: app.fetch,
    port,
  })
}
