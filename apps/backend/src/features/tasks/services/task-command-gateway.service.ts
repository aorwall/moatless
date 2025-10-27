import type { Command } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { CommandService } from '../../../services/command.service'
import { filterAvailableCommands } from '../../../utils/command-conditions'
import { FileGateway } from '../../workspace/services/file-gateway.service'
import { TaskMetadataStorage } from '../storage/task-metadata.storage'
import { deriveTaskStatus } from '../utils/task-status.utils'

/**
 * TaskCommandGateway
 *
 * Service for filtering available commands based on workspace state.
 * Builds command context from task state, git status, and iteration history
 * to determine which commands are currently available.
 *
 * @responsibilities
 * - Query available commands for a task
 * - Build command context from workspace state
 * - Apply command condition filtering
 *
 * @dependencies
 * - CommandService - Get all defined commands
 * - FileGateway - Get git status and workspace state
 * - TaskMetadataStorage - Get task data
 */
@injectable()
export class TaskCommandGateway {
  constructor(
    @inject(CommandService) private commandService: CommandService,
    @inject(FileGateway) private fileGateway: FileGateway,
    @inject(TaskMetadataStorage) private taskStorage: TaskMetadataStorage,
  ) {}

  /**
   * Get available commands for a task
   *
   * Filters the full command list based on:
   * - Task status
   * - Last iteration status
   * - Git status (uncommitted changes, commits ahead/behind)
   * - Command conditions
   *
   * @param taskId - Task ID to get commands for
   * @returns Filtered list of available commands
   * @throws TaskNotFoundError if task doesn't exist
   */
  async getAvailableCommands(taskId: string): Promise<Command[]> {
    // Get task data
    const task = await this.taskStorage.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Derive status from last iteration
    const taskWithStatus = deriveTaskStatus(task)

    // Get all commands
    const allCommands = await this.commandService.listCommands()

    // Get git status to check for uncommitted changes
    const gitStatus = await this.fileGateway.getGitStatus(taskId)
    const workspaceHasChanges = gitStatus?.hasUncommittedChanges

    // Get last iteration
    const lastIteration =
      taskWithStatus.iterations && taskWithStatus.iterations.length > 0
        ? taskWithStatus.iterations[taskWithStatus.iterations.length - 1]
        : undefined

    // Build command context
    const context = {
      ...(taskWithStatus !== undefined && { task: taskWithStatus }),
      ...(lastIteration !== undefined && { lastIteration }),
      ...(workspaceHasChanges !== undefined && { workspaceHasChanges }),
      ...(gitStatus !== undefined && { gitStatus }),
    }

    // Filter commands based on conditions
    return filterAvailableCommands(allCommands, context)
  }
}
