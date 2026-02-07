import crypto from 'node:crypto'
import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type { AskUserQuestionResponse, Event, ExitPlanModeResponse } from '@moatless/types'
import type { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import type { AppBindings } from '../../../app'
import { createBackendApiClient } from '../../../shared/backend-api-client'
import { logger } from '../../../shared/logger'
import { createSandboxTools } from '../../tools'
import type { SandboxTool } from '../../tools/types'
import { ClaudeCodeAgent } from '../claude-code/claude-code-agent'
import {
  getActivePromptToolName,
  hasActivePrompt,
  resolveInteractivePrompt,
} from '../claude-code/interactive-prompt'

// Module-scoped agent state
let currentAgent: ClaudeCodeAgent | undefined
let agentState: 'idle' | 'running' | 'stopped' = 'idle'
let lastError: string | undefined

// Lazy-initialized tools (created once per sandbox lifetime)
let tools: SandboxTool[] | undefined

/**
 * Get the current agent snapshot (used by GET /status)
 */
export function getAgentSnapshot() {
  return {
    state: agentState,
    hasAgent: currentAgent !== undefined,
    error: lastError,
  }
}

/**
 * Register agent-related routes
 * Agent execution is handled through these endpoints
 */
export function registerAgentRoutes(app: Hono<AppBindings>) {
  // POST /execute-agent - Start agent execution
  // Returns JSON acknowledgment immediately, runs agent in background
  // Use /agent-status to check completion, or /stream-events for SSE
  // Accepts AgentRunRequest directly (content array with text/images, options)
  app.post('/execute-agent', async (c) => {
    try {
      const request = await c.req.json<AgentRunRequest>()

      // Validate required content
      if (!request.content || request.content.length === 0) {
        return c.json({ success: false, error: 'content is required' }, 400)
      }

      if (agentState === 'running') {
        return c.json({ success: false, error: 'Agent is already running' }, 409)
      }

      const config = c.get('config')
      const workspaceDir = c.get('workspaceDir')
      const eventService = c.get('eventService')

      // Lazy-init tools
      if (!tools) {
        tools = createSandboxTools(config, createBackendApiClient(config))
      }

      // Create new agent
      currentAgent = new ClaudeCodeAgent(config.taskId, {
        model: request.model,
        sandboxTools: tools,
      })
      agentState = 'running'
      lastError = undefined

      // Extract message text for logging
      const messageText = request.content
        .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
        .map((c) => c.text)
        .join('\n')

      logger.info(
        {
          taskId: config.taskId,
          messageLength: messageText.length,
          model: request.model,
          hasEventService: !!eventService,
          resume: request.resume,
          agentMode: request.agentMode,
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
          // Event callback passed separately (not part of serializable request)
          const onEvent = async (event: Event) => {
            if (eventService) {
              try {
                await eventService.emitEvent(event)
              } catch (emitError) {
                logger.warn(
                  { err: emitError, taskId: config.taskId, eventType: event.type },
                  'Failed to emit event to Redis',
                )
              }
            }
            logger.debug({ taskId: config.taskId, eventType: event.type }, 'Agent event processed')
          }

          // Pass request and workspaceDir (derived from sandbox context) separately
          await agent.run(request, workspaceDir, onEvent)

          logger.info({ taskId: config.taskId }, 'Agent execution completed successfully')
        } catch (error) {
          lastError = error instanceof Error ? error.message : String(error)
          logger.error({ err: error, taskId: config.taskId }, 'Agent execution error')
          // Emit error event so backend can handle the failure
          if (eventService) {
            try {
              await eventService.emitEvent({
                eventId: crypto.randomUUID(),
                type: 'agent',
                subType: 'error',
                taskId: config.taskId,
                agentType: 'claude-code',
                timestamp: new Date(),
                isError: true,
                payload: { error: lastError },
              } as Event)
            } catch (emitError) {
              logger.warn(
                { err: emitError, taskId: config.taskId },
                'Failed to emit error event to Redis',
              )
            }
          }
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
  // Accepts AgentRunRequest directly (content array with text/images, options)
  app.post('/stream-events', async (c) => {
    try {
      const request = await c.req.json<AgentRunRequest>()

      // Validate required content
      if (!request.content || request.content.length === 0) {
        return c.json({ success: false, error: 'content is required' }, 400)
      }

      if (agentState === 'running') {
        return c.json({ success: false, error: 'Agent is already running' }, 409)
      }

      const config = c.get('config')
      const workspaceDir = c.get('workspaceDir')

      // Lazy-init tools
      if (!tools) {
        tools = createSandboxTools(config, createBackendApiClient(config))
      }

      // Create new agent
      currentAgent = new ClaudeCodeAgent(config.taskId, {
        model: request.model,
        sandboxTools: tools,
      })
      agentState = 'running'
      lastError = undefined

      // Extract message text for logging
      const messageText = request.content
        .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
        .map((c) => c.text)
        .join('\n')

      logger.info(
        {
          taskId: config.taskId,
          messageLength: messageText.length,
          model: request.model,
          resume: request.resume,
          agentMode: request.agentMode,
        },
        'Starting agent execution (streaming)',
      )

      // Stream events via SSE
      // Backend handles iteration completion by subscribing to agent.completed events
      // Capture agent reference for use in async callback (avoids non-null assertion)
      const agent = currentAgent
      return streamSSE(c, async (stream) => {
        try {
          // Event callback passed separately (not part of serializable request)
          const onEvent = async (event: Event) => {
            await stream.writeSSE({
              event: event.type,
              data: JSON.stringify(event),
            })
          }

          // Pass request and workspaceDir (derived from sandbox context) separately
          await agent.run(request, workspaceDir, onEvent)

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

      if (!hasActivePrompt()) {
        return c.json({ success: false, error: 'No pending user input request' }, 404)
      }

      const pendingTool = getActivePromptToolName()
      if (pendingTool !== body.toolName) {
        return c.json(
          {
            success: false,
            error: `Tool mismatch: expected ${pendingTool}, got ${body.toolName}`,
          },
          400,
        )
      }

      const resolved = resolveInteractivePrompt(body.response)

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

  // POST /notify-messages - Wake up the message polling consumer
  app.post('/notify-messages', (c) => {
    const consumer = c.get('messageConsumer')
    if (consumer) {
      consumer.notify()
    }
    return c.json({ success: true })
  })

  // POST /command - Execute a command (placeholder for now)
  app.post('/command', (c) =>
    c.json({ exit_code: 1, stdout: '', stderr: 'Not implemented yet' }, 501),
  )
}
