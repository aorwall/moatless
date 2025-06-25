import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { SDKMessage } from '@anthropic-ai/claude-code'
import type { ClaudeCodeSettings } from '@shared/types/claude.js'
import { type Event, EventType } from '@shared/types/event.js'
import type { Task } from '@shared/types/task.js'

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
    type: EventType.MESSAGE,
    taskId,
    iterationId: iterationNumber,
    timestamp: new Date(),
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
    return null
  }

  // Get the last iteration with a session ID
  const lastIteration = task.iterations
    .slice()
    .reverse()
    .find((iter) => iter.agentSessionId)

  if (!lastIteration?.agentSessionId) {
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

    for (const line of lines) {
      try {
        const sdkMessage = JSON.parse(line) as SDKMessage
        const event = sdkMessageToEvent(sdkMessage, task.taskId, lastIteration?.iterationNumber)
        events.push(event)
      } catch (parseError) {
        console.error('Failed to parse SDK message:', parseError)
      }
    }

    return events
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
      // File doesn't exist, return empty array
      return []
    }
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
