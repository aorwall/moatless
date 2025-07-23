import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { SDKMessage } from '@anthropic-ai/claude-code'
import type { AgentEvent, ClaudeCodeSettings, Event, Task } from '@moatless/types'

import { calculateContextTokens } from './context-tokens.js'
import { logger } from './logger.js'

/**
 * Convert SDK message to Event format
 */
export function sdkMessageToEvent(
  sdkMessage: SDKMessage,
  taskId: string,
  iterationNumber?: number,
): Event {
  // Check if this is an error message
  const isError = 'error' in sdkMessage && sdkMessage.error !== undefined

  // Calculate context tokens using types function
  const contextTokens = calculateContextTokens(sdkMessage)

  if (sdkMessage.type === 'result') {
    return {
      eventId: crypto.randomUUID(),
      agentType: 'claude-code',
      type: 'agent',
      subType: 'completed',
      taskId,
      iterationId: iterationNumber,
      isError,
      contextTokens,
      payload: sdkMessage,
    } as AgentEvent
  }

  return {
    eventId: crypto.randomUUID(),
    agentType: 'claude-code',
    type: 'agent',
    subType: 'message',
    taskId,
    iterationId: iterationNumber,
    isError,
    contextTokens,
    payload: sdkMessage,
  }
}

/**
 * Generate project ID from workspace path
 */
export function generateProjectId(workspacePath: string): string {
  return workspacePath.replace(/[^a-zA-Z0-9]/g, '-')
}

/**
 * Get Claude Code events file path for a specific iteration
 */
export function getClaudeEventsFilePath(
  settings: ClaudeCodeSettings,
  workspacePath: string,
  iteration: Task['iterations'][0],
): string | null {
  if (!iteration.agentSessionId) {
    return null
  }

  const projectId = generateProjectId(workspacePath)
  return path.join(
    settings.claudeConfigPath,
    'projects',
    projectId,
    `${iteration.agentSessionId}.jsonl`,
  )
}

/**
 * Read Claude Code events from disk
 */
export async function readClaudeCodeEvents(
  settings: ClaudeCodeSettings,
  task: Task,
): Promise<Event[]> {
  if (!task.workspacePath) {
    logger.warn('No workspace path found for task', {
      taskId: task.taskId,
    })
    return []
  }

  // Try iterations in reverse order
  for (const iteration of [...task.iterations].reverse()) {
    const filePath = getClaudeEventsFilePath(settings, task.workspacePath, iteration)
    if (!filePath) {
      continue
    }

    try {
      const fileContent = await fs.readFile(filePath, 'utf-8')
      const lines = fileContent.trim().split('\n').filter(Boolean)

      const events: Event[] = []
      let _parseErrors = 0

      for (const line of lines) {
        try {
          const sdkMessage = JSON.parse(line) as SDKMessage
          const event = sdkMessageToEvent(sdkMessage, task.taskId, iteration.iterationNumber)
          events.push(event)
        } catch (parseError) {
          _parseErrors++
          logger.warn('Failed to parse Claude Code SDK message', {
            taskId: task.taskId,
            error: parseError instanceof Error ? parseError.message : String(parseError),
          })
        }
      }

      return events
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        logger.debug('Claude Code events file does not exist, trying next iteration', {
          taskId: task.taskId,
          filePath,
          iterationNumber: iteration.iterationNumber,
        })
        continue
      }

      logger.error('Error reading Claude Code events file', error as Error, {
        taskId: task.taskId,
        filePath,
      })
      throw error
    }
  }

  logger.warn('No Claude Code events files found for any iteration', {
    taskId: task.taskId,
    totalIterations: task.iterations.length,
    iterationsWithSessionIds: task.iterations.filter((iter) => iter.agentSessionId).length,
  })
  return []
}

/**
 * Get default Claude Code settings
 */
export function getDefaultClaudeCodeSettings(): ClaudeCodeSettings {
  return {
    claudeConfigPath: process.env.CLAUDE_CONFIG_DIR || path.join(process.env.HOME || '', '.claude'),
  }
}
