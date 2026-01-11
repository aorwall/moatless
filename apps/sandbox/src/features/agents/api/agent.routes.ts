import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { ClaudeCodeAgent } from '../claude-code/claude-code-agent'

// Module-scoped agent state
let currentAgent: ClaudeCodeAgent | undefined
let agentState: 'idle' | 'running' | 'stopped' = 'idle'

/**
 * Register agent-related routes
 * Agent execution is handled through these endpoints
 */
export function registerAgentRoutes(app: Hono<AppBindings>) {
  // POST /execute-agent - Start agent execution (SSE stream)
  app.post('/execute-agent', async (c) => {
    try {
      const body = await c.req.json<{
        message: string
        model?: string
        timeout?: number
        resume?: string
        agentMode?: 'edit' | 'plan'
        appendSystemPrompt?: string
      }>()

      if (!body.message) {
        return c.json({ success: false, error: 'message is required' }, 400)
      }

      if (agentState === 'running') {
        return c.json({ success: false, error: 'Agent is already running' }, 409)
      }

      const config = c.get('config')
      const workspaceDir = c.get('workspaceDir')

      // Create new agent
      currentAgent = new ClaudeCodeAgent(config.taskId, {
        model: body.model,
      })
      agentState = 'running'

      logger.info(
        {
          taskId: config.taskId,
          messageLength: body.message.length,
          model: body.model,
        },
        'Starting agent execution',
      )

      // Stream events via SSE
      return streamSSE(c, async (stream) => {
        try {
          for await (const event of currentAgent!.run(body.message, {
            workspaceDir,
            timeout: body.timeout,
            resume: body.resume,
            agentMode: body.agentMode,
            appendSystemPrompt: body.appendSystemPrompt,
          })) {
            await stream.writeSSE({
              event: event.type,
              data: JSON.stringify(event),
            })
          }

          // Send completion event
          await stream.writeSSE({
            event: 'done',
            data: JSON.stringify({ type: 'done', completed: true }),
          })
        } catch (error) {
          logger.error({ err: error }, 'Agent execution error')
          await stream.writeSSE({
            event: 'error',
            data: JSON.stringify({
              type: 'error',
              error: error instanceof Error ? error.message : String(error),
            }),
          })
        } finally {
          agentState = 'stopped'
        }
      })
    } catch (error) {
      agentState = 'idle'
      logger.error({ err: error }, 'Failed to start agent execution')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // POST /stop-agent - Stop running agent
  app.post('/stop-agent', async (c) => {
    try {
      if (!currentAgent || agentState !== 'running') {
        return c.json({ success: true, message: 'No agent running' })
      }

      await currentAgent.stop()
      agentState = 'stopped'

      return c.json({ success: true, stopped: true })
    } catch (error) {
      logger.error({ err: error }, 'Failed to stop agent')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // GET /agent-status - Get current agent status
  app.get('/agent-status', (c) => {
    return c.json({
      state: agentState,
      hasAgent: currentAgent !== undefined,
    })
  })

  // POST /command - Execute a command (placeholder for now)
  app.post('/command', (c) =>
    c.json({ exit_code: 1, stdout: '', stderr: 'Not implemented yet' }, 501),
  )
}
