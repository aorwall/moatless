import type { Hono } from 'hono'
import type { AppBindings } from '../app.js'

export function registerWorkerRoutes(app: Hono<AppBindings>) {
  // Middleware to authenticate worker requests
  const authenticateWorker = async (c: any, next: any) => {
    const authHeader = c.req.header('Authorization')
    if (!authHeader?.startsWith('Bearer ')) {
      return c.json({ error: 'Unauthorized' }, 401)
    }

    // const apiKey = authHeader.slice(7)
    // TODO: Validate API key against registered workers

    await next()
  }

  // Task-specific event forwarding endpoint
  app.post('/api/v1/tasks/:taskId/worker/events', authenticateWorker, async (c) => {
    const taskId = c.req.param('taskId')
    const event = await c.req.json()

    // Ensure event has the correct taskId
    event.taskId = taskId

    // Forward to event service
    await c.get('services').event.emitEvent(event)

    return c.json({ success: true })
  })

  // Task-specific worker registration endpoint
  app.post('/api/v1/tasks/:taskId/worker/register', async (c) => {
    const taskId = c.req.param('taskId')
    await c.req.json() // Worker info for future use

    // TODO: Register worker in worker service with taskId association

    return c.json({
      success: true,
      taskId,
      apiKey: `worker-${taskId}-${Date.now()}`,
    })
  })

  // Agent execution endpoint for workers
  app.post('/api/v1/tasks/:taskId/worker/execute-agent', authenticateWorker, async (c) => {
    const taskId = c.req.param('taskId')
    const { agentType, message, iterationNumber, resumeSessionId, model } = await c.req.json()

    // Get agent service and execute
    const agentService = c.get('services').agent

    // Create iteration object for executeAgent
    const iteration = {
      iterationNumber,
      startTime: new Date(),
      status: 'active' as const, // Note: TaskIteration uses 'active', not 'running'
      costUsd: 0,
      durationMs: 0,
      messageCount: 1, // Required property
    }

    // Execute agent using the correct method name
    await agentService.executeAgent(taskId, message, iteration, agentType, resumeSessionId, model)

    return c.json({ success: true })
  })

  // Agent stop endpoint for workers
  app.post('/api/v1/tasks/:taskId/worker/stop-agent', authenticateWorker, async (c) => {
    const taskId = c.req.param('taskId')

    // Get services
    const agentService = c.get('services').agent
    const workspaceService = c.get('services').workspace

    // Stop the agent first
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
      console.warn('Could not stop workspace agent:', error)
    }

    return c.json({
      success: true,
      agentStopped,
      workspaceStopped,
    })
  })
}
