import type { Command } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { CommandService } from '../../../services/command.service'
import { filterAvailableCommands } from '../../../utils/command-conditions'
import { FileGateway } from '../../workspace/services/file-gateway.service'
import { TaskLifecycleService } from './task-lifecycle.service'

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
 * - TaskLifecycleService - Get task and iteration data
 */
@injectable()
export class TaskCommandGateway {
  constructor(
    @inject(CommandService) private commandService: CommandService,
    @inject(FileGateway) private fileGateway: FileGateway,
    @inject(TaskLifecycleService) private lifecycleService: TaskLifecycleService,
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
   */
  async getAvailableCommands(taskId: string): Promise<Command[]> {
    // Get task data
    const task = await this.lifecycleService.getTask(taskId)

    // Get all commands
    const allCommands = await this.commandService.listCommands()

    // Get git status to check for uncommitted changes
    const gitStatus = await this.fileGateway.getGitStatus(taskId)
    const workspaceHasChanges = gitStatus?.hasUncommittedChanges

    // Get last iteration
    const lastIteration =
      task.iterations && task.iterations.length > 0
        ? task.iterations[task.iterations.length - 1]
        : undefined

    // Build command context
    const context = {
      ...(task !== undefined && { task }),
      ...(lastIteration !== undefined && { lastIteration }),
      ...(workspaceHasChanges !== undefined && { workspaceHasChanges }),
      ...(gitStatus !== undefined && { gitStatus }),
    }

    // Filter commands based on conditions
    return filterAvailableCommands(allCommands, context)
  }
}
