import type { Event } from '@moatless/types'
import { z } from 'zod/v4'
import type { SandboxConfig } from '../../config/environment'
import { logger } from '../../shared/logger'
import type { SandboxEventService } from '../events/services/sandbox-event.service'
import type { SandboxTool } from './types'

const toolLogger = logger.child({ service: 'tool-update-task-metadata' })

/**
 * Create the updateTaskMetadata tool.
 *
 * Emits a `metadataUpdated` event via SandboxEventService — same pattern
 * as TaskMetadataMonitor.emitMetadataEvent().
 */
export function createUpdateTaskMetadataTool(
  config: SandboxConfig,
  eventService: SandboxEventService | undefined,
): SandboxTool {
  return {
    name: 'updateTaskMetadata',
    description:
      'Update the task name and/or description. Use this to set a concise title and summary for the current task.',
    schema: {
      name: z.string().optional().describe('Short task name / title'),
      description: z.string().optional().describe('Task description or summary'),
    },
    execute: async (args) => {
      if (!(args.name || args.description)) {
        return {
          text: 'At least one of name or description must be provided.',
          isError: true,
        }
      }

      toolLogger.info(
        { taskId: config.taskId, name: args.name, hasDescription: !!args.description },
        'updateTaskMetadata called',
      )

      if (eventService) {
        try {
          const event = {
            taskId: config.taskId,
            type: 'task',
            subType: 'metadataUpdated',
            payload: {
              name: args.name,
              description: args.description,
            },
            isError: false,
          } as unknown as Event

          await eventService.emitEvent(event)
          toolLogger.debug({ taskId: config.taskId }, 'Metadata event emitted')
        } catch (emitError) {
          toolLogger.warn({ err: emitError }, 'Failed to emit metadata event')
          return {
            text: 'Failed to update task metadata.',
            isError: true,
          }
        }
      }

      return {
        text: 'Task metadata updated successfully.',
      }
    },
  }
}
