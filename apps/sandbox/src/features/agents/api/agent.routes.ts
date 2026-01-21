import { existsSync } from 'node:fs'
import path from 'node:path'
import type { AskUserQuestionResponse, Event, ExitPlanModeResponse } from '@moatless/types'
import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import {
  ClaudeCodeAgent,
  getPendingToolName,
  hasPendingUserInput,
  resolveUserInput,
} from '../claude-code/claude-code-agent'

// Module-scoped agent state
let currentAgent: ClaudeCodeAgent | undefined
let agentState: 'idle' | 'running' | 'stopped' = 'idle'
let lastError: string | undefined

/**
 * Check if preview server is available in the workspace
 * Returns true if package.json exists (indicates Node.js project)
 */
function checkHasPreviewServer(workspaceDir: string): boolean {
  const packageJsonPath = path.join(workspaceDir, 'package.json')
  return existsSync(packageJsonPath)
}

/**
 * Register agent-related routes
 * Agent execution is handled through these endpoints
 */
export function registerAgentRoutes(app: Hono<AppBindings>) {
  // POST /execute-agent - Start agent execution
  // Returns JSON acknowledgment immediately, runs agent in background
  // Use /agent-status to check completion, or /stream-events for SSE
  app.post('/execute-agent', async (c) => {
    try {
      const body = await c.req.json<{
        message: string
        model?: string
        taskId?: string
        agentType?: string
        runOptions?: {
          timeout?: number
          resume?: string
          agentMode?: 'edit' | 'plan'
          appendSystemPrompt?: string
        }
      }>()

      if (!body.message) {
        return c.json({ success: false, error: 'message is required' }, 400)
      }

      if (agentState === 'running') {
        return c.json({ success: false, error: 'Agent is already running' }, 409)
      }

      const config = c.get('config')
      const workspaceDir = c.get('workspaceDir')
      const eventService = c.get('eventService')

      // Determine if preview server is available by checking for package.json
      // This indicates a Node.js project with potentially a dev server
      const hasPreviewServer = await checkHasPreviewServer(workspaceDir)

      // Create new agent
      currentAgent = new ClaudeCodeAgent(config.taskId, {
        model: body.model,
      })
      agentState = 'running'
      lastError = undefined

      logger.info(
        {
          taskId: config.taskId,
          messageLength: body.message.length,
          model: body.model,
          hasPreviewServer,
          hasEventService: !!eventService,
        },
        'Starting agent execution',
      )

      // Start agent execution in background (non-blocking)
      // Events are emitted to Redis for SSE streaming to frontend
      // Backend handles iteration completion by subscribing to agent.completed events
      // Capture agent reference for use in async callback (avoids non-null assertion)
      const agent = currentAgent
      ;(async () => {
        try {
          // Consume all events from the agent
          for await (const event of agent.run(body.message, {
            workspaceDir,
            timeout: body.runOptions?.timeout,
            resume: body.runOptions?.resume,
            agentMode: body.runOptions?.agentMode,
            appendSystemPrompt: body.runOptions?.appendSystemPrompt,
            hasPreviewServer,
          })) {
            // Emit event to Redis for SSE streaming (if eventService available)
            if (eventService) {
              try {
                await eventService.emitEvent(event as Event)
              } catch (emitError) {
                logger.warn(
                  { err: emitError, taskId: config.taskId, eventType: event.type },
                  'Failed to emit event to Redis',
                )
              }
            }

            logger.debug({ taskId: config.taskId, eventType: event.type }, 'Agent event processed')
          }

          logger.info({ taskId: config.taskId }, 'Agent execution completed successfully')
        } catch (error) {
          lastError = error instanceof Error ? error.message : String(error)
          logger.error({ err: error, taskId: config.taskId }, 'Agent execution error')
        } finally {
          agentState = 'idle'
        }
      })().catch((err) => {
        logger.error({ err }, 'Unhandled error in background agent execution')
      })

      // Return acknowledgment immediately
      return c.json({ success: true })
    } catch (error) {
      agentState = 'idle'
      lastError = error instanceof Error ? error.message : String(error)
      logger.error({ err: error }, 'Failed to start agent execution')
      return c.json(
        {
          success: false,
          error: lastError,
        },
        500,
      )
    }
  })

  // POST /stream-events - Stream agent events via SSE (optional endpoint for direct SSE consumers)
  app.post('/stream-events', async (c) => {
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

      // Determine if preview server is available by checking for package.json
      // This indicates a Node.js project with potentially a dev server
      const hasPreviewServer = await checkHasPreviewServer(workspaceDir)

      // Create new agent
      currentAgent = new ClaudeCodeAgent(config.taskId, {
        model: body.model,
      })
      agentState = 'running'
      lastError = undefined

      logger.info(
        {
          taskId: config.taskId,
          messageLength: body.message.length,
          model: body.model,
          hasPreviewServer,
        },
        'Starting agent execution (streaming)',
      )

      // Stream events via SSE
      // Backend handles iteration completion by subscribing to agent.completed events
      // Capture agent reference for use in async callback (avoids non-null assertion)
      const agent = currentAgent
      return streamSSE(c, async (stream) => {
        try {
          for await (const event of agent.run(body.message, {
            workspaceDir,
            timeout: body.timeout,
            resume: body.resume,
            agentMode: body.agentMode,
            appendSystemPrompt: body.appendSystemPrompt,
            hasPreviewServer,
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
          lastError = error instanceof Error ? error.message : String(error)
          logger.error({ err: error }, 'Agent execution error')
          await stream.writeSSE({
            event: 'error',
            data: JSON.stringify({
              type: 'error',
              error: lastError,
            }),
          })
        } finally {
          agentState = 'idle'
        }
      })
    } catch (error) {
      agentState = 'idle'
      lastError = error instanceof Error ? error.message : String(error)
      logger.error({ err: error }, 'Failed to start agent execution')
      return c.json(
        {
          success: false,
          error: lastError,
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
      error: lastError,
    })
  })

  // POST /user-input-response - Submit user response for AskUserQuestion or ExitPlanMode
  app.post('/user-input-response', async (c) => {
    try {
      const body = await c.req.json<{
        toolName: 'AskUserQuestion' | 'ExitPlanMode'
        response: AskUserQuestionResponse | ExitPlanModeResponse
      }>()

      if (!body.toolName || body.response === undefined) {
        return c.json({ success: false, error: 'toolName and response are required' }, 400)
      }

      if (!hasPendingUserInput()) {
        return c.json({ success: false, error: 'No pending user input request' }, 404)
      }

      const pendingTool = getPendingToolName()
      if (pendingTool !== body.toolName) {
        return c.json(
          {
            success: false,
            error: `Tool mismatch: expected ${pendingTool}, got ${body.toolName}`,
          },
          400,
        )
      }

      const resolved = resolveUserInput(body.response)

      if (!resolved) {
        return c.json({ success: false, error: 'Failed to resolve user input' }, 500)
      }

      logger.info({ toolName: body.toolName }, 'User input resolved')
      return c.json({ success: true })
    } catch (error) {
      logger.error({ err: error }, 'Failed to process user input response')
      return c.json(
        {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // POST /command - Execute a command (placeholder for now)
  app.post('/command', (c) =>
    c.json({ exit_code: 1, stdout: '', stderr: 'Not implemented yet' }, 501),
  )
}
