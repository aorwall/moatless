import { serve } from '@hono/node-server'
import type { TaskIteration } from '@moatless/shared/types/task'
import { Hono } from 'hono'
import { AgentService } from './services/agent.service.js'
import { RemoteEventService } from './services/remote-event.service.js'
import { RemoteStorageBackend } from './services/remote-storage.service.js'
import { WorkspaceService } from './services/workspace.service.js'
import { createLogger } from './utils/logger.js'
import { LocalWorkspace } from './workspaces/local.js'

export interface WorkerModeConfig {
  apiUrl: string
  apiKey: string
  workspaceDir: string
}

export async function startWorkerMode(config: WorkerModeConfig): Promise<void> {
  const logger = createLogger()
  logger.info('Starting backend in worker mode', {
    apiUrl: config.apiUrl,
    workspaceDir: config.workspaceDir,
  })

  // Initialize services that proxy to main backend
  const eventService = new RemoteEventService(config.apiUrl, config.apiKey)
  const storage = new RemoteStorageBackend(config.apiUrl, config.apiKey)

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
  const agentService = new AgentService(
    storage,
    eventService as any,
    workspaceService,
    null, // No repository service needed in worker mode
  )

  // Create HTTP server
  const app = new Hono()

  // Health check endpoint
  app.get('/health', (c) =>
    c.json({
      status: 'ok',
      mode: 'worker',
      workspaceDir: config.workspaceDir,
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

  const port = parseInt(process.env.PORT || '8001')
  logger.info(`Worker HTTP server starting on port ${port}`)

  serve({
    fetch: app.fetch,
    port,
  })
}
