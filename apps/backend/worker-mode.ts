import { serve } from '@hono/node-server'
import type { TaskIteration } from '@moatless/shared/types/task'
import { Hono } from 'hono'
import { AgentService } from './services/agent.service.js'
import { FileStorage } from './services/storage.js'
import { WorkerEventService } from './services/worker-event.service.js'
import { WorkspaceService } from './services/workspace.service.js'
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
  })

  // Initialize services for worker mode
  const taskId = process.env.WORKER_TASK_ID

  // WorkerEventService forwards events to main backend through task-specific endpoints
  const eventService = new WorkerEventService(config.apiUrl, config.apiKey, taskId)

  // Use regular FileStorage with mounted volume path
  // The data directory is mounted from the host, allowing shared storage
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

  // SSE event stream endpoint - streams events in real-time
  app.get('/events/stream', async (c) => {
    const taskId = process.env.WORKER_TASK_ID

    if (!taskId) {
      return c.json({ error: 'WORKER_TASK_ID not set' }, 500)
    }

    logger.info('Client connected to worker event stream', { taskId })

    // Create a streaming response
    const { readable, writable } = new TransformStream()
    const writer = writable.getWriter()

    // Track last event timestamp
    let lastEventTimestamp: Date | undefined
    let isStreaming = true

    // Poll for new events periodically
    const pollEvents = async () => {
      while (isStreaming) {
        try {
          // Get all events from storage
          const allEvents = await storage.getEvents(taskId)

          // Filter new events
          let newEvents = allEvents
          if (lastEventTimestamp) {
            newEvents = allEvents.filter((event) => {
              const eventTime = event.timestamp ? new Date(event.timestamp) : null
              return eventTime && eventTime > lastEventTimestamp!
            })
          }

          // Send new events
          for (const event of newEvents) {
            await writer.write(new TextEncoder().encode(`${JSON.stringify(event)}\n`))

            // Update last timestamp
            const eventTime = event.timestamp ? new Date(event.timestamp) : null
            if (eventTime && (!lastEventTimestamp || eventTime > lastEventTimestamp)) {
              lastEventTimestamp = eventTime
            }
          }

          // Wait before next poll
          await new Promise((resolve) => setTimeout(resolve, 500))
        } catch (error) {
          logger.error('Error polling events', error as Error)
          break
        }
      }

      await writer.close()
    }

    // Start polling
    pollEvents()

    // Send periodic pings to keep connection alive
    const pingInterval = setInterval(async () => {
      try {
        const pingEvent = {
          type: 'ping',
          timestamp: new Date().toISOString(),
        }
        await writer.write(new TextEncoder().encode(`${JSON.stringify(pingEvent)}\n`))
      } catch (_error) {
        clearInterval(pingInterval)
      }
    }, 30000)

    // Handle client disconnect
    c.req.raw.signal?.addEventListener('abort', () => {
      logger.info('Client disconnected from worker event stream', { taskId })
      isStreaming = false
      clearInterval(pingInterval)
      writer.close()
    })

    return new Response(readable, {
      status: 200,
      headers: {
        'Content-Type': 'text/event-stream',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
        'X-Accel-Buffering': 'no', // Disable Nginx buffering
      },
    })
  })

  const port = parseInt(process.env.PORT || '8001')
  logger.info(`Worker HTTP server starting on port ${port}`)

  serve({
    fetch: app.fetch,
    port,
  })
}
