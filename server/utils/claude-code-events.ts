import type { SDKMessage } from '@anthropic-ai/claude-code'
import type { ClaudeCodeSettings } from '@shared/types/claude.js'
import { type Event, EventType } from '@shared/types/event.js'
import type { Task } from '@shared/types/task.js'
import { promises as fs } from 'node:fs'
import path from 'node:path'
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

  return {
    eventId: crypto.randomUUID(),
    agentType: 'claude-code',
    type: sdkMessage.type === 'result' ? EventType.RESULT : EventType.MESSAGE,
    taskId,
    iterationId: iterationNumber,
    isError,
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
 * Get Claude Code events file path
 */
export function getClaudeEventsFilePath(settings: ClaudeCodeSettings, task: Task): string | null {
  if (!task.workspacePath) {
    logger.warn('No workspace path for Claude Code events', { taskId: task.taskId })
    return null
  }

  // Get the last iteration with a session ID
  const lastIteration = task.iterations
    .slice()
    .reverse()
    .find((iter) => iter.agentSessionId)

  if (!lastIteration?.agentSessionId) {
    logger.warn('No iteration with Claude Code session ID found', {
      taskId: task.taskId,
      totalIterations: task.iterations.length,
      iterationsWithSessionIds: task.iterations.filter((iter) => iter.agentSessionId).length,
    })
    return null
  }

  const projectId = generateProjectId(task.workspacePath)
  const sessionId = lastIteration.agentSessionId
  return path.join(settings.claudeConfigPath, 'projects', projectId, `${sessionId}.jsonl`)
}

/**
 * Read Claude Code events from disk
 */
export async function readClaudeCodeEvents(
  settings: ClaudeCodeSettings,
  task: Task,
): Promise<Event[]> {
  const filePath = getClaudeEventsFilePath(settings, task)
  if (!filePath) {
    logger.warn('Cannot read Claude Code events - no file path available', {
      taskId: task.taskId,
    })
    return []
  }

  try {
    const fileContent = await fs.readFile(filePath, 'utf-8')
    const lines = fileContent.trim().split('\n').filter(Boolean)

    const events: Event[] = []
    const lastIteration = task.iterations
      .slice()
      .reverse()
      .find((iter) => iter.agentSessionId)

    let parseErrors = 0
    for (const line of lines) {
      try {
        const sdkMessage = JSON.parse(line) as SDKMessage
        const event = sdkMessageToEvent(sdkMessage, task.taskId, lastIteration?.iterationNumber)
        events.push(event)
      } catch (parseError) {
        parseErrors++
        logger.warn('Failed to parse Claude Code SDK message', {
          taskId: task.taskId,
          error: parseError instanceof Error ? parseError.message : String(parseError),
        })
      }
    }

    logger.debug('Successfully read Claude Code events', {
      taskId: task.taskId,
      filePath,
      totalLines: lines.length,
      eventsRead: events.length,
      parseErrors,
    })

    return events
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
      logger.debug('Claude Code events file does not exist', {
        taskId: task.taskId,
        filePath,
      })
      return []
    }

    logger.error('Error reading Claude Code events file', error as Error, {
      taskId: task.taskId,
      filePath,
    })
    throw error
  }
}

/**
 * Get default Claude Code settings
 */
export function getDefaultClaudeCodeSettings(): ClaudeCodeSettings {
  return {
    claudeConfigPath: process.env.CLAUDE_CONFIG_DIR || path.join(process.env.HOME || '', '.claude'),
  }
}
