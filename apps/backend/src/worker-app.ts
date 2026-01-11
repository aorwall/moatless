import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { serve } from '@hono/node-server'
import type { RepositoryConfig } from '@moatless/types'
import { Hono } from 'hono'
import type { AgentRunOptions, BaseAgent } from './features/agents/agent/base-agent'
import type { EventService } from './features/events/services/event.service'
import { PreviewServer } from './features/preview/runtime/preview-server.js'
import type {
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerSnapshot,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
} from './features/preview/types/index.js'
import { LocalEnvironmentDriver } from './features/workspace/drivers/local-environment-driver'
import { cloneRepository } from './features/workspace/utils/git.utils'
import { logger as loggerMiddleware } from './middleware/logger'
import { requestId } from './middleware/request-id'
import { logger } from './utils/logger'

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

function isPostgresUrl(databaseUrl?: string): boolean {
  return Boolean(databaseUrl && /^postgres(ql)?:\/\//i.test(databaseUrl))
}

async function initializeWorkerEventService(dataDir?: string): Promise<EventService> {
  const databaseUrl = process.env['DATABASE_URL']
  if (!databaseUrl) {
    throw new Error('DATABASE_URL is required in worker mode to emit events')
  }
  if (!isPostgresUrl(databaseUrl)) {
    throw new Error('DATABASE_URL must be a PostgreSQL connection string in worker mode')
  }

  if (process.env['DATABASE_PROVIDER'] !== 'postgres') {
    if (process.env['DATABASE_PROVIDER']) {
      logger.warn(
        { databaseProvider: process.env['DATABASE_PROVIDER'] },
        'Overriding DATABASE_PROVIDER to postgres for worker event delivery',
      )
    }
    process.env['DATABASE_PROVIDER'] = 'postgres'
  }

  const { createPostgresDatabase } = await import('./infrastructure/database/postgres')
  const { FileStorage } = await import('./infrastructure/storage/file-storage')
  const { EventsStorage } = await import('./features/events/storage/events.storage')
  const { EventRepository } = await import('./features/events/db/event.repository')
  const { EventService } = await import('./features/events/services/event.service')

  const database = createPostgresDatabase(databaseUrl)
  const storage = new FileStorage(dataDir || '/data')
  const eventsStorage = new EventsStorage(storage)
  const eventRepository = new EventRepository(database)
  const eventService = new EventService(eventsStorage, eventRepository)

  logger.info('Worker EventService initialized with PostgreSQL delivery')

  return eventService
}

/**
 * Lifecycle sink for PreviewServer in worker mode.
 * Emits events via EventService (PostgreSQL-backed, same as agent events).
 *
 * Note: register/update/remove are no-ops because state is managed by the main app's
 * orchestrator (which computes proxyUrl from workerUrl). The lifecycle sink is
 * primarily used for event emission to SSE clients.
 */
class WorkerPreviewLifecycleSink implements PreviewServerLifecycleSink {
  constructor(private readonly eventService: EventService) {}

  async register(_serverId: string, _snapshot: PreviewServerSnapshot): Promise<void> {
    // No-op: State managed by main app orchestrator (needs proxyUrl computation)
  }

  async update(_serverId: string, _changes: PreviewServerUpdate): Promise<void> {
    // No-op: State managed by main app orchestrator
  }

  async remove(_serverId: string): Promise<void> {
    // No-op: State managed by main app orchestrator
  }

  async onStatusChange(serverId: string, statusInfo: ServerStatusChangeInfo): Promise<void> {
    const subType = statusInfo.status as 'starting' | 'started' | 'stopped' | 'failed'

    const event = {
      eventId: randomUUID(),
      type: 'server' as const,
      subType,
      taskId: serverId,
      timestamp: new Date(),
      isError: statusInfo.status === 'failed',
      payload: {
        status: statusInfo.status,
        port: statusInfo.port,
        pid: statusInfo.pid,
        url: statusInfo.externalUrl,
        error: statusInfo.error,
        uptimeSeconds: statusInfo.uptimeSeconds,
      },
    }

    logger.info({ serverId, status: statusInfo.status }, 'Emitting server status event')

    try {
      await this.eventService.emitEvent(event)
    } catch (error) {
      logger.error(
        { err: error as Error, serverId, status: statusInfo.status },
        'Failed to emit server event',
      )
    }
  }
}

/**
 * Run agent in background and emit events via EventService (PostgreSQL-backed).
 *
 * Events are persisted in PostgreSQL and delivered to the main API via NOTIFY/LISTEN
 * (or the poller fallback if NOTIFY fails).
 */
async function runAgentInBackground(
  agent: BaseAgent,
  message: string,
  runOptions: AgentRunOptions,
  taskId: string,
  eventService: EventService,
  setAgentState: (state: AgentState) => void,
): Promise<void> {
  setAgentState('running')

  try {
    // Execute agent and emit events
    for await (const event of agent.run(message, runOptions)) {
      logger.debug({ taskId, eventType: event.type }, 'Agent event')

      try {
        await eventService.emitEvent(event)
      } catch (error) {
        logger.error({ err: error as Error, taskId, eventType: event.type }, 'Failed to emit event')
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
 * - EventService to emit events via PostgreSQL (NOTIFY/LISTEN)
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
  const environmentDriver = new LocalEnvironmentDriver(taskId, config.workspaceDir)
  const eventService = await initializeWorkerEventService(config.dataDir)

  // Create lifecycle sink for preview server event emission
  const previewLifecycleSink = new WorkerPreviewLifecycleSink(eventService)

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

      const agent = await environmentDriver.createAgent(agentType || 'claude-code', agentOptions)

      // Run agent in background and emit events via EventService (PostgreSQL-backed)
      runAgentInBackground(agent, message, adjustedRunOptions, taskId, eventService, setAgentState)

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
      await environmentDriver.stopAgent()
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
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex endpoint with validation, setup, lifecycle, and error handling
  app.post('/server/start', async (c) => {
    const body = await c.req.json()
    const { serverCommand, serverCommands, envVars, setupCommands, startupTimeoutSeconds } = body

    try {
      const requestedCommands = Array.isArray(serverCommands) ? serverCommands : []
      const fallbackCommands = repositoryConfig?.previewConfig?.serverStartCommands || []
      const resolvedCommands =
        requestedCommands.length > 0
          ? requestedCommands
          : serverCommand
            ? [serverCommand]
            : fallbackCommands

      if (resolvedCommands.length === 0) {
        return c.json(
          {
            success: false,
            error: 'No server command configured',
          },
          400,
        )
      }

      // Create preview server config directly (feature module doesn't need adapter)
      // Use setupCommands from request first, fall back to repository config
      const previewConfig: PreviewServerConfig = {
        serverId: taskId || 'worker',
        workspacePath: config.workspaceDir,
        setupCommands: setupCommands || repositoryConfig?.codeExecution?.setupCommands || [],
        serverCommands: resolvedCommands,
        envVars: envVars || repositoryConfig?.codeExecution?.envVars || {},
        startupTimeoutSeconds:
          startupTimeoutSeconds ?? repositoryConfig?.previewConfig?.startupTimeoutSeconds,
        // Disable inspector/proxy in worker - proxying handled by /server/proxy endpoint
        enableInspector: false,
      }

      // Create preview server instance with config and lifecycle sink
      previewServer = new PreviewServer(previewConfig, previewLifecycleSink)

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

      for await (const event of environmentDriver.runCommand(command, runOptions)) {
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
      const content = await environmentDriver.readFile(filePath)
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

  // Write file endpoint
  interface WriteFileRequest {
    filePath: string
    content: string
  }

  app.post('/write-file', async (c) => {
    const body = await c.req.json<WriteFileRequest>()
    const { filePath, content } = body

    logger.debug({ filePath, taskId }, 'Writing file')

    try {
      await environmentDriver.writeFile(filePath, content)
      return c.json({ success: true })
    } catch (error) {
      logger.error({ err: error as Error, filePath }, 'Failed to write file')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : 'Failed to write file',
        },
        500,
      )
    }
  })

  // List files endpoint
  interface ListFilesRequest {
    relativePath?: string
    recursive?: boolean
  }

  app.post('/list-files', async (c) => {
    const body = await c.req.json<ListFilesRequest>()
    const { relativePath, recursive } = body

    logger.debug({ relativePath, recursive, taskId }, 'Listing files')

    try {
      const files = await environmentDriver.listFiles(relativePath, recursive)
      return c.json({ files })
    } catch (error) {
      logger.error({ err: error as Error, relativePath }, 'Failed to list files')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Failed to list files',
        },
        500,
      )
    }
  })

  // Search files endpoint
  interface SearchFilesRequest {
    query: string
    limit?: number
  }

  app.post('/search-files', async (c) => {
    const body = await c.req.json<SearchFilesRequest>()
    const { query, limit } = body

    logger.debug({ query, limit, taskId }, 'Searching files')

    try {
      const searchRequest = { query, ...(limit !== undefined && { limit }) }
      const result = await environmentDriver.searchFiles(searchRequest)
      return c.json(result)
    } catch (error) {
      logger.error({ err: error as Error, query }, 'Failed to search files')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Failed to search files',
        },
        500,
      )
    }
  })

  // Git status endpoint
  app.get('/git-status', async (c) => {
    logger.debug({ taskId }, 'Getting git status')

    try {
      const status = await environmentDriver.getGitStatus()
      return c.json(status)
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to get git status')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Failed to get git status',
        },
        500,
      )
    }
  })

  // Git changes endpoint
  app.get('/git-changes', async (c) => {
    logger.debug({ taskId }, 'Getting git changes')

    try {
      const changes = await environmentDriver.getGitChanges()
      return c.json(changes)
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to get git changes')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Failed to get git changes',
        },
        500,
      )
    }
  })

  // ============================================================================
  // Extended Git Operations (for file tree and diff views)
  // ============================================================================

  // File tree with git changes endpoint
  interface FileTreeRequest {
    rootPath?: string
  }

  app.post('/file-tree-with-changes', async (c) => {
    const body = await c.req.json<FileTreeRequest>()
    const { rootPath } = body

    logger.debug({ taskId, rootPath }, 'Getting file tree with changes')

    try {
      const result = await environmentDriver.getFileTreeWithChanges(rootPath)
      return c.json(result)
    } catch (error) {
      logger.error({ err: error as Error, rootPath }, 'Failed to get file tree with changes')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Failed to get file tree',
        },
        500,
      )
    }
  })

  // Git diff endpoint
  app.get('/git-diff', async (c) => {
    logger.debug({ taskId }, 'Getting git diff')

    try {
      const result = await environmentDriver.getGitDiff()
      return c.json(result)
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to get git diff')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Failed to get git diff',
        },
        500,
      )
    }
  })

  // Read file with changes endpoint
  interface ReadFileWithChangesRequest {
    filePath: string
  }

  app.post('/read-file-with-changes', async (c) => {
    const body = await c.req.json<ReadFileWithChangesRequest>()
    const { filePath } = body

    logger.debug({ taskId, filePath }, 'Reading file with changes')

    try {
      const result = await environmentDriver.readFileWithChanges(filePath)
      return c.json(result)
    } catch (error) {
      logger.error({ err: error as Error, filePath }, 'Failed to read file with changes')
      return c.json(
        {
          error: error instanceof Error ? error.message : 'Failed to read file',
        },
        500,
      )
    }
  })

  // Current commit hash endpoint
  app.get('/current-commit-hash', async (c) => {
    logger.debug({ taskId }, 'Getting current commit hash')

    try {
      const hash = await environmentDriver.getCurrentCommitHash()
      return c.json({ hash })
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to get commit hash')
      return c.json({ hash: null }, 200) // Return null on error, not 500
    }
  })

  const port = Number.parseInt(process.env['PORT'] || '8001')
  logger.info(`Worker HTTP server starting on port ${port}`)

  serve({
    fetch: app.fetch,
    port,
  })
}
