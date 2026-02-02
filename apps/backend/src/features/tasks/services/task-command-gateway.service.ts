import type { Command } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { filterAvailableCommands } from '../../../utils/command-conditions'
import { CommandService } from '../../agents/services/command.service'
import { TaskRepository } from '../db/task.repository'

/**
 * TaskCommandGateway
 *
 * Service for filtering available commands based on task state.
 * Builds command context from task data to determine which commands
 * are currently available.
 *
 * @responsibilities
 * - Query available commands for a task
 * - Build command context from task state
 * - Apply command condition filtering
 *
 * @dependencies
 * - CommandService - Get all defined commands
 * - TaskRepository - Get task data
 */
@injectable()
export class TaskCommandGateway {
  constructor(
    @inject(CommandService) private commandService: CommandService,
    @inject(TaskRepository) private taskRepository: TaskRepository,
  ) {}

  /**
   * Get available commands for a task
   *
   * Filters the full command list based on:
   * - Task status
   * - Last iteration status
   * - Command conditions
   *
   * @param taskId - Task ID to get commands for
   * @returns Filtered list of available commands
   * @throws TaskNotFoundError if task doesn't exist
   */
  async getAvailableCommands(taskId: string): Promise<Command[]> {
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    const allCommands = await this.commandService.listCommands()

    return filterAvailableCommands(allCommands, { task })
  }
}
