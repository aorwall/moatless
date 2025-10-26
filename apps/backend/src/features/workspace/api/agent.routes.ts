import type { OpenAPIHono } from '@hono/zod-openapi'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { streamSSE } from '../../../utils/sse-stream'
import { AgentController } from '../services/agent-controller.service'

/**
 * Register agent routes using new workspace feature architecture
 *
 * These routes use AgentController which:
 * - Calls WorkspaceCoordinator.ensureWorkspace() internally
 * - Delegates to LocalEnvironmentDriver for agent creation
 * - Updates runtime snapshots with agent state
 */
export function registerAgentRoutes(app: OpenAPIHono<AppBindings>) {
  // POST /api/v1/tasks/:taskId/agent/start
  // Note: Using non-OpenAPI endpoint for now since agent API contracts may need updating
  app.post('/api/v1/tasks/:taskId/agent/start', async (c) => {
    const agentController = appContainer.get(AgentController)
    const taskId = c.req.param('taskId')

    if (!taskId) {
      return c.json({ error: 'Task ID is required' }, 400)
    }

    // Parse request body
    const body = await c.req.json()
    const { agentType = 'claude-code', message, runOptions } = body

    if (!message) {
      return c.json({ error: 'Message is required' }, 400)
    }

    // Transform agent events to SSE format
    async function* transformToSSE() {
      for await (const event of agentController.startAgent(
        taskId,
        agentType,
        message,
        runOptions,
      )) {
        yield {
          data: JSON.stringify(event),
          type: event.type,
        }
      }
    }

    return streamSSE(c, transformToSSE())
  })

  // POST /api/v1/tasks/:taskId/agent/stop
  app.post('/api/v1/tasks/:taskId/agent/stop', async (c) => {
    const agentController = appContainer.get(AgentController)
    const taskId = c.req.param('taskId')

    if (!taskId) {
      return c.json({ error: 'Task ID is required' }, 400)
    }

    await agentController.stopAgent(taskId)
    return c.json({ stopped: true })
  })

  // GET /api/v1/tasks/:taskId/agent/status
  app.get('/api/v1/tasks/:taskId/agent/status', async (c) => {
    const agentController = appContainer.get(AgentController)
    const taskId = c.req.param('taskId')

    if (!taskId) {
      return c.json({ error: 'Task ID is required' }, 400)
    }

    const status = await agentController.getStatus(taskId)
    return c.json(status)
  })
}
