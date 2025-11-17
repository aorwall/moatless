import { promises as fs } from 'node:fs'
import path from 'node:path'
import { serve } from '@hono/node-server'
import type { RepositoryConfig } from '@moatless/types'
import { Hono } from 'hono'
// import { LocalEnvironment } from './environments/local-environment' // REMOVED - LocalEnvironment is deprecated
import { PreviewServer } from './features/preview/runtime/preview-server.js'
import type { PreviewServerConfig } from './features/preview/types/index.js'
import { logger as loggerMiddleware } from './middleware/logger'
import { requestId } from './middleware/request-id'
// import type { AgentRunOptions, BaseAgent } from './services/index' // Unused - worker mode deprecated
import { logger } from './utils/logger'

// import { WorkerApiClient } from './utils/worker-api-client' // Unused - worker mode deprecated

interface WorkerModeConfig {
  apiUrl: string
  apiKey?: string
  workspaceDir: string
  dataDir?: string // Optional data directory for storage (defaults to /data)
}

// Worker mode is deprecated - this function is no longer used
// /**
//  * Run agent in background and emit events to main API
//  */
// async function runAgentInBackground(
//   agent: BaseAgent,
//   message: string,
//   runOptions: AgentRunOptions,
//   taskId: string,
//   apiUrl: string,
//   apiKey: string | undefined,
//   logger: ReturnType<typeof createLogger>,
// ): Promise<void> {
//   const apiClient = new WorkerApiClient(apiUrl, apiKey)

//   try {
//     // Execute agent and emit events to main API
//     for await (const event of agent.run(message, runOptions)) {
//       logger.info({  taskId, event  }, 'Agent event')

//       try {
//         await apiClient.emitEvent(taskId, event)
//       } catch (error) {
//         logger.error({ err: error as Error,
//           taskId,
//           event,
//          }, 'Failed to emit event to main API')
//         // Continue processing even if event emission fails
//       }
//     }

//     logger.info({  taskId  }, 'Agent execution completed')
//   } catch (error) {
//     logger.error({ err: error as Error,  taskId  }, 'Agent execution failed')
//     // Agent will emit its own error events, so we don't need to emit one here
//   }
// }

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
  logger.info(
    {
      apiUrl: config.apiUrl,
      workspaceDir: config.workspaceDir,
      dataDir: config.dataDir || '/data',
      logLevel: process.env['LOG_LEVEL'] || 'info',
    },
    'Starting backend in worker mode',
  )

  if (!process.env['WORKER_TASK_ID']) {
    throw new Error('WORKER_TASK_ID environment variable not set')
  }

  // Initialize services for worker mode
  const taskId = process.env['WORKER_TASK_ID']

  // Read repository configuration from mounted file
  let repositoryConfig: RepositoryConfig
  const configPath = path.join(config.dataDir || '/data', 'repository-config.json')

  try {
    const configData = await fs.readFile(configPath, 'utf-8')
    repositoryConfig = JSON.parse(configData) as RepositoryConfig
    logger.info(
      {
        configPath,
        repositoryId: repositoryConfig.id,
        repositoryName: repositoryConfig.name,
      },
      'Loaded repository configuration from file',
    )
  } catch (error) {
    logger.error({ err: error as Error, configPath }, 'Failed to load repository configuration')
    throw new Error(`Repository configuration not found at ${configPath}`)
  }

  // FIXME: Worker mode needs refactoring to use WorkspaceRuntimeManager
  // LocalEnvironment has been deprecated and removed. Worker mode is currently non-functional.
  // TODO: Refactor worker mode to use:
  // - WorkspaceRuntimeManager for workspace provisioning
  // - PreviewOrchestratorService for preview server management
  // - LocalEnvironmentDriver for agent execution
  //
  // const environment = new LocalEnvironment(
  //   taskId,
  //   repositoryConfig,
  //   config.workspaceDir,
  //   undefined,
  //   undefined,
  //   undefined,
  // )

  // Store single agent instance for this worker (only one agent can run at a time)
  // let agentInstance: BaseAgent | null = null // Unused - worker mode deprecated

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
      taskId: taskId,
    }),
  )

  // Agent execution endpoint - returns immediately and runs agent in background
  // interface ExecuteAgentRequest { // Unused - worker mode deprecated
  //   taskId: string
  //   message: string
  //   agentType?: string
  //   runOptions?: AgentRunOptions
  //   model?: string
  // }

  app.post('/execute-agent', (c) => {
    // const body = await c.req.json<ExecuteAgentRequest>()
    // const { taskId, message, agentType, runOptions, model } = body

    // logger.info({
    //   taskId,
    //   agentType,
    //   workspaceDir: runOptions?.workspaceDir,
    //   model,
    //  }, 'Received agent execution request')

    try {
      // // Ensure runOptions uses the worker's actual workspace directory
      // const adjustedRunOptions: AgentRunOptions = {
      //   ...(runOptions ?? {}),
      //   workspaceDir: config.workspaceDir,
      //   ...(model ? { model } : {}),
      // }

      // agentInstance = await environment.createAgent(
      //   agentType || 'claude-code',
      //   model ? { model } : {},
      // )
      throw new Error(
        'Worker mode is currently non-functional - LocalEnvironment has been deprecated. Needs refactoring to use WorkspaceRuntimeManager.',
      )

      // // Run agent in background and emit events to main API
      // runAgentInBackground(
      //   agentInstance,
      //   message,
      //   adjustedRunOptions,
      //   taskId,
      //   config.apiUrl,
      //   config.apiKey,
      //   logger,
      // )

      // return c.json({
      //   success: true,
      //   message: 'Agent execution started',
      // })
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Failed to create agent')
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
  app.post('/stop-agent', (c) => {
    // const body = await c.req.json()
    // const { taskId } = body

    // logger.info({  taskId  }, 'Received agent stop request')

    try {
      throw new Error(
        'Worker mode is currently non-functional - LocalEnvironment has been deprecated',
      )

      // // Get stored agent instance
      // let agentStopped = false

      // if (agentInstance) {
      //   // Stop the agent instance
      //   await agentInstance.stop()
      //   agentStopped = true

      //   // Clear the agent instance
      //   agentInstance = null
      //   logger.info({  taskId  }, 'Agent instance stopped and removed')
      // } else {
      //   logger.warn({  taskId  }, 'No agent instance found to stop')
      // }

      // logger.info({  taskId, agentStopped  }, 'Agent stop completed')
      // return c.json({
      //   success: true,
      //   agentStopped,
      // })
    } catch (error) {
      logger.error({ err: error as Error }, 'Agent stop failed')
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
      // Use server command from request or fall back to repository config
      const finalServerCommand =
        serverCommand || repositoryConfig?.previewConfig?.serverStartCommands?.[0]

      if (!finalServerCommand) {
        return c.json(
          {
            success: false,
            error: 'No server command configured',
          },
          400,
        )
      }

      // Create preview server config directly (feature module doesn't need adapter)
      const previewConfig: PreviewServerConfig = {
        serverId: taskId || 'worker',
        workspacePath: config.workspaceDir,
        setupCommands: repositoryConfig.codeExecution.setupCommands || [],
        serverCommands: [finalServerCommand],
        envVars: envVars || repositoryConfig?.codeExecution?.envVars || {},
        startupTimeoutSeconds: repositoryConfig?.previewConfig?.startupTimeoutSeconds,
        enableInspector: repositoryConfig?.previewConfig?.enableInspector,
      }

      // Create preview server instance with config
      previewServer = new PreviewServer(previewConfig)

      await previewServer.start()

      // Get status after start
      const result = previewServer.getStatus()

      // Extract port from internal URL for backwards compatibility
      const port = result.internalUrl
        ? Number.parseInt(new URL(result.internalUrl).port || '0', 10)
        : undefined

      logger.info(
        {
          status: result.status,
          port,
          pid: result.pid,
        },
        'Preview server started',
      )

      return c.json({
        success: true,
        status: result.status,
        port: port || 0,
        pid: result.pid || 0,
        url: result.internalUrl, // Worker API returns internal URL
        uptimeSeconds: result.uptimeSeconds || 0,
      })
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to start preview server')
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

      logger.info({ success: stopped }, 'Preview server stopped')

      return c.json({
        success: stopped,
      })
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to stop preview server')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  app.get('/server/status', (c) => {
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
      logger.error({ err: error as Error }, 'Failed to get server status')
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
      logger.error({ err: error as Error }, 'Failed to get server logs')
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

      const bodyText =
        c.req.method !== 'GET' && c.req.method !== 'HEAD' ? await c.req.text() : undefined
      const result = await previewServer.proxyRequest({
        method: c.req.method,
        path: fullPath,
        headers,
        ...(bodyText !== undefined && { body: bodyText }),
      })

      // Return the proxied response
      return new Response(result.body, {
        status: result.status,
        headers: result.headers,
      })
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to proxy request')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        502,
      )
    }
  })

  // Command execution endpoint
  app.post('/command', (c) => {
    // const body = await c.req.json()
    // const { command, timeout } = body

    try {
      // const result = await environment.runCommand(command, timeout)
      throw new Error(
        'Worker mode is currently non-functional - LocalEnvironment has been deprecated',
      )

      // return c.json({
      //   exit_code: result.exitCode || 0,
      //   stdout: result.output || '',
      //   stderr: '',
      // })
    } catch (error) {
      logger.error({ err: error as Error }, 'Command execution failed')
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

  const port = Number.parseInt(process.env['PORT'] || '8001')
  logger.info(`Worker HTTP server starting on port ${port}`)

  serve({
    fetch: app.fetch,
    port,
  })
}
