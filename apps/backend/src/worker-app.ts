import { promises as fs } from 'node:fs'
import path from 'node:path'
import { serve } from '@hono/node-server'
import type { RepositoryConfig } from '@moatless/types'
import { Hono } from 'hono'
import type { AgentRunOptions, BaseAgent } from './features/agents/agent/base-agent'
import { PreviewServer } from './features/preview/runtime/preview-server.js'
import type { PreviewServerConfig } from './features/preview/types/index.js'
import { LocalEnvironmentDriver } from './features/workspace/drivers/local-environment-driver'
import { cloneRepository } from './features/workspace/utils/git.utils'
import { logger as loggerMiddleware } from './middleware/logger'
import { requestId } from './middleware/request-id'
import { logger } from './utils/logger'
import { WorkerApiClient } from './utils/worker-api-client'

interface WorkerModeConfig {
  apiUrl: string
  workspaceDir: string
  dataDir?: string // Optional data directory for storage (defaults to /data)
  gitCloneUrl?: string // Git repository URL to clone on startup
  gitBranch?: string // Git branch to checkout
}

/**
 * Agent execution state for tracking running agents
 */
type AgentState = 'idle' | 'running' | 'completed'

/**
 * Run agent in background and emit events to main API
 */
async function runAgentInBackground(
  agent: BaseAgent,
  message: string,
  runOptions: AgentRunOptions,
  taskId: string,
  apiClient: WorkerApiClient,
  setAgentState: (state: AgentState) => void,
): Promise<void> {
  setAgentState('running')

  try {
    // Execute agent and emit events to main API
    for await (const event of agent.run(message, runOptions)) {
      logger.debug({ taskId, eventType: event.type }, 'Agent event')

      try {
        await apiClient.emitEvent(taskId, event)
      } catch (error) {
        logger.error(
          { err: error as Error, taskId, eventType: event.type },
          'Failed to emit event to main API',
        )
        // Continue processing even if event emission fails
      }
    }

    logger.info({ taskId }, 'Agent execution completed')
    setAgentState('completed')
  } catch (error) {
    logger.error({ err: error as Error, taskId }, 'Agent execution failed')
    setAgentState('idle')
  }
}

/**
 * Start the backend in worker mode
 *
 * Worker mode is used when running the backend inside a Docker/Kubernetes container
 * to execute agent tasks. It uses:
 * - WorkerApiClient to forward events to the main backend
 * - LocalEnvironmentDriver for agent execution within the container
 * - Git clone on startup for Kubernetes mode
 */
export async function startWorkerMode(config: WorkerModeConfig): Promise<void> {
  logger.info(
    {
      apiUrl: config.apiUrl,
      workspaceDir: config.workspaceDir,
      dataDir: config.dataDir || '/data',
      gitCloneUrl: config.gitCloneUrl,
      gitBranch: config.gitBranch,
      logLevel: process.env['LOG_LEVEL'] || 'info',
    },
    'Starting backend in worker mode',
  )

  if (!process.env['WORKER_TASK_ID']) {
    throw new Error('WORKER_TASK_ID environment variable not set')
  }

  const taskId = process.env['WORKER_TASK_ID']

  // Clone repository if git URL is provided (Kubernetes mode)
  if (config.gitCloneUrl) {
    logger.info(
      {
        gitCloneUrl: config.gitCloneUrl,
        gitBranch: config.gitBranch,
        targetDir: config.workspaceDir,
      },
      'Cloning repository for Kubernetes worker',
    )

    try {
      const cloneOptions: { branch?: string; depth?: number } = {}
      if (config.gitBranch) {
        cloneOptions.branch = config.gitBranch
      }

      await cloneRepository(config.gitCloneUrl, config.workspaceDir, cloneOptions)
      logger.info({ workspaceDir: config.workspaceDir }, 'Repository cloned successfully')
    } catch (error) {
      logger.error(
        { err: error as Error, gitCloneUrl: config.gitCloneUrl },
        'Failed to clone repository',
      )
      throw new Error(
        `Failed to clone repository: ${error instanceof Error ? error.message : 'Unknown error'}`,
      )
    }
  }

  // Read repository configuration from mounted file (Docker mode fallback)
  let repositoryConfig: RepositoryConfig | undefined
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
  } catch {
    // Repository config is optional in Kubernetes mode
    if (!config.gitCloneUrl) {
      logger.warn({ configPath }, 'No repository configuration found and no git URL provided')
    } else {
      logger.debug({ configPath }, 'No repository configuration file (expected in Kubernetes mode)')
    }
  }

  // Initialize services
  const environmentDriver = new LocalEnvironmentDriver()
  const apiClient = new WorkerApiClient(config.apiUrl)

  // Agent state tracking
  let agentState: AgentState = 'idle'
  const setAgentState = (state: AgentState) => {
    agentState = state
  }

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
  interface ExecuteAgentRequest {
    taskId: string
    message: string
    agentType?: string
    runOptions?: AgentRunOptions
    model?: string
  }

  app.post('/execute-agent', async (c) => {
    const body = await c.req.json<ExecuteAgentRequest>()
    const { message, agentType, runOptions, model } = body

    logger.info(
      {
        taskId,
        agentType,
        model,
      },
      'Received agent execution request',
    )

    try {
      // Ensure runOptions uses the worker's actual workspace directory
      const adjustedRunOptions: AgentRunOptions = {
        ...(runOptions ?? {}),
        workspaceDir: config.workspaceDir,
        ...(model ? { model } : {}),
      }

      // Create agent using LocalEnvironmentDriver
      const agentOptions: { model?: string } = {}
      if (model) {
        agentOptions.model = model
      }

      const agent = environmentDriver.createAgent(taskId, agentType || 'claude-code', agentOptions)

      // Run agent in background and emit events to main API
      runAgentInBackground(agent, message, adjustedRunOptions, taskId, apiClient, setAgentState)

      return c.json({
        success: true,
        message: 'Agent execution started',
      })
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
  app.post('/stop-agent', async (c) => {
    logger.info({ taskId }, 'Received agent stop request')

    try {
      await environmentDriver.stopAgent(taskId)
      setAgentState('idle')

      logger.info({ taskId }, 'Agent stop completed')
      return c.json({
        success: true,
        agentStopped: true,
      })
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Agent stop failed')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        },
        500,
      )
    }
  })

  // Agent status endpoint
  app.get('/agent-status', (c) => {
    return c.json({
      state: agentState,
    })
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
        setupCommands: repositoryConfig?.codeExecution?.setupCommands || [],
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
  interface CommandRequest {
    command: string
    timeout?: number
    env?: Record<string, string>
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex command processing with streaming output
  app.post('/command', async (c) => {
    const body = await c.req.json<CommandRequest>()
    const { command, timeout, env } = body

    logger.info({ command, timeout, taskId }, 'Executing command')

    try {
      // Collect all output from the command
      let stdout = ''
      let stderr = ''
      let exitCode = 0

      const runOptions: { timeout?: number; env?: Record<string, string> } = {}
      if (timeout !== undefined) {
        runOptions.timeout = timeout
      }
      if (env !== undefined) {
        runOptions.env = env
      }

      for await (const event of environmentDriver.runCommand(
        config.workspaceDir,
        command,
        runOptions,
      )) {
        if (event.type === 'stdout') {
          stdout += event.text
        } else if (event.type === 'stderr') {
          stderr += event.text
        } else if (event.type === 'system') {
          // Parse exit code from system message if present
          const match = event.text.match(/exit(?:ed with)? code (\d+)/i)
          if (match?.[1]) {
            exitCode = Number.parseInt(match[1], 10)
          }
        }
      }

      return c.json({
        exit_code: exitCode,
        stdout,
        stderr,
      })
    } catch (error) {
      logger.error({ err: error as Error, command }, 'Command execution failed')
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

  // Read file endpoint
  interface ReadFileRequest {
    filePath: string
  }

  app.post('/read-file', async (c) => {
    const body = await c.req.json<ReadFileRequest>()
    const { filePath } = body

    logger.debug({ filePath, taskId }, 'Reading file')

    try {
      const content = await environmentDriver.readFile(config.workspaceDir, filePath)
      return c.json({ content })
    } catch (error) {
      logger.error({ err: error as Error, filePath }, 'Failed to read file')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'File not found',
        },
        404,
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
