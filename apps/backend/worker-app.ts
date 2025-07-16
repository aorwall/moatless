import { serve } from '@hono/node-server'
import { Hono } from 'hono'
import { LocalEnvironment } from './environments/local-environment.js'
import { logger as loggerMiddleware } from './middleware/logger.js'
import { requestId } from './middleware/request-id.js'
import { AgentService, WorkspaceService } from './services/index.js'
import { PreviewServer } from './services/preview-server.js'
import { FileStorage } from './services/storage.js'
import { WorkerEventService } from './services/worker-event.service.js'
import { createLogger } from './utils/logger.js'
import { Workspace } from './workspaces/workspace.js'

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
 * - Workspace with LocalEnvironment for agent execution within the container
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

  // Create workspace service that uses Workspace with LocalEnvironment
  const workspaceService = new WorkspaceService(null as any, eventService as any, storage)

  // Override workspace factory to always use LocalEnvironment in worker mode
  workspaceService.createWorkspace = async (taskId: string, repositoryConfig: any) => {
    const environment = new LocalEnvironment(
      taskId,
      repositoryConfig,
      eventService as any,
      config.workspaceDir,
    )
    return new Workspace(
      taskId,
      repositoryConfig,
      eventService as any,
      environment,
      config.workspaceDir,
    )
  }

  // Create agent service
  const agentService = new AgentService(storage, eventService as any, workspaceService)

  // Store agent instances for this worker
  const agentInstances: Map<string, { agent: any; agentType: string; createdAt: Date }> = new Map()

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

  // Create agent endpoint (for Docker environment)
  app.post('/create-agent', async (c) => {
    const body = await c.req.json()
    const { taskId, agentType, options } = body

    logger.info('Received agent creation request', {
      taskId,
      agentType,
    })

    try {
      // Get workspace to create agent
      const workspace = await workspaceService.getWorkspace(taskId)
      if (!workspace) {
        return c.json(
          {
            success: false,
            error: 'Workspace not found',
          },
          404,
        )
      }

      // Get environment from workspace
      const environment = workspace.getEnvironment()

      // Create agent using environment
      const agent = await environment.createAgent(agentType || 'claude-code', options)

      // Store agent instance
      agentInstances.set(taskId, {
        agent,
        agentType: agentType || 'claude-code',
        createdAt: new Date(),
      })

      logger.info('Agent created successfully', { taskId, agentType })

      return c.json({
        success: true,
        agentType: agentType || 'claude-code',
        sessionId: agent.getSessionId(),
      })
    } catch (error) {
      logger.error('Agent creation failed', error as Error, { taskId })
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  // Agent execution endpoint with SSE streaming
  app.post('/execute-agent', async (c) => {
    const body = await c.req.json()
    const { taskId, message, agentType, runOptions } = body

    logger.info('Received agent execution request', {
      taskId,
      agentType,
      workspaceDir: runOptions?.workspaceDir,
    })

    // Set up SSE response
    c.header('Content-Type', 'text/event-stream')
    c.header('Cache-Control', 'no-cache')
    c.header('Connection', 'keep-alive')

    const encoder = new TextEncoder()
    const stream = new ReadableStream({
      async start(controller) {
        try {
          // Get workspace to execute agent
          const workspace = await workspaceService.getWorkspace(taskId)
          if (!workspace) {
            controller.enqueue(
              encoder.encode(
                `data: ${JSON.stringify({ type: 'error', message: 'Workspace not found' })}\n\n`,
              ),
            )
            controller.enqueue(encoder.encode('data: [DONE]\n\n'))
            controller.close()
            return
          }

          // Get stored agent instance
          const agentData = agentInstances.get(taskId)
          if (!agentData) {
            controller.enqueue(
              encoder.encode(
                `data: ${JSON.stringify({ type: 'error', message: 'Agent not found. Create agent first.' })}\n\n`,
              ),
            )
            controller.enqueue(encoder.encode('data: [DONE]\n\n'))
            controller.close()
            return
          }

          const { agent } = agentData

          // Get environment from workspace
          const environment = workspace.getEnvironment()

          // Execute agent and stream events
          for await (const event of environment.executeAgent(agent, message, runOptions || {})) {
            const data = `data: ${JSON.stringify(event)}\n\n`
            controller.enqueue(encoder.encode(data))

            // Update session ID if captured
            if (event.type === 'session' && event.sessionId) {
              agent.sessionId = event.sessionId
            }
          }

          // Send completion signal
          controller.enqueue(encoder.encode('data: [DONE]\n\n'))
          controller.close()

          logger.info('Agent execution completed', { taskId })
        } catch (error) {
          logger.error('Agent execution failed', error as Error, { taskId })
          const errorEvent = {
            type: 'error',
            message: error instanceof Error ? error.message : 'Unknown error',
          }
          controller.enqueue(encoder.encode(`data: ${JSON.stringify(errorEvent)}\n\n`))
          controller.enqueue(encoder.encode('data: [DONE]\n\n'))
          controller.close()
        }
      },
    })

    return new Response(stream)
  })

  // Stop agent endpoint
  app.post('/stop-agent', async (c) => {
    const body = await c.req.json()
    const { taskId } = body

    logger.info('Received agent stop request', { taskId })

    try {
      // Get stored agent instance
      const agentData = agentInstances.get(taskId)
      let agentStopped = false

      if (agentData) {
        // Stop the agent instance
        await agentData.agent.stop()
        agentStopped = true

        // Remove from instances map
        agentInstances.delete(taskId)
        logger.info('Agent instance stopped and removed', { taskId })
      }

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

  // Get agent session ID endpoint
  app.get('/agent-session/:taskId', async (c) => {
    const { taskId } = c.req.param()

    try {
      const agentData = agentInstances.get(taskId)
      if (!agentData) {
        return c.json(
          {
            success: false,
            error: 'Agent not found',
          },
          404,
        )
      }

      return c.json({
        success: true,
        sessionId: agentData.agent.getSessionId(),
        agentType: agentData.agentType,
        createdAt: agentData.createdAt,
      })
    } catch (error) {
      logger.error('Failed to get agent session', error as Error, { taskId })
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  // Send message to agent endpoint
  app.post('/agent-message', async (c) => {
    const body = await c.req.json()
    const { taskId, message, options } = body

    try {
      const agentData = agentInstances.get(taskId)
      if (!agentData) {
        return c.json(
          {
            success: false,
            error: 'Agent not found',
          },
          404,
        )
      }

      const response = await agentData.agent.sendMessage(message, options)

      return c.json({
        success: true,
        response,
      })
    } catch (error) {
      logger.error('Failed to send message to agent', error as Error, { taskId })
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  // Get branch name and summary from agent endpoint
  app.post('/agent-branch-summary', async (c) => {
    const body = await c.req.json()
    const { taskId, message, options } = body

    try {
      const agentData = agentInstances.get(taskId)
      if (!agentData) {
        return c.json(
          {
            success: false,
            error: 'Agent not found',
          },
          404,
        )
      }

      const result = await agentData.agent.getBranchNameAndSummary(message, options)

      return c.json({
        success: true,
        result,
      })
    } catch (error) {
      logger.error('Failed to get branch summary from agent', error as Error, { taskId })
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

  // Proxy requests to the preview server
  app.all('/server/proxy/*', async (c) => {
    try {
      if (!previewServer) {
        return c.json(
          {
            error: 'No server running',
          },
          404,
        )
      }

      // Get the path after /server/proxy
      const proxyPath = c.req.path.replace('/server/proxy', '') || '/'
      const queryString = c.req.url.split('?')[1] || ''
      const fullPath = queryString ? `${proxyPath}?${queryString}` : proxyPath

      const headers = new Headers()
      // Copy relevant headers
      const headersToForward = ['content-type', 'accept', 'user-agent', 'authorization', 'cookie']
      for (const [key, value] of Object.entries(c.req.header())) {
        if (headersToForward.includes(key.toLowerCase())) {
          headers.set(key, value)
        }
      }

      const result = await previewServer.proxyRequest({
        method: c.req.method,
        path: fullPath,
        headers,
        body: c.req.method !== 'GET' && c.req.method !== 'HEAD' ? await c.req.text() : undefined,
      })

      // Return the proxied response
      return new Response(result.body, {
        status: result.status,
        headers: result.headers,
      })
    } catch (error) {
      logger.error('Failed to proxy request', error as Error)
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        502,
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
