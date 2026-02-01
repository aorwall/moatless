import type { Command } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { filterAvailableCommands } from '../../../utils/command-conditions'
import { CommandService } from '../../agents/services/command.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskRepository } from '../db/task.repository'

/** Built-in commands injected based on workspace state (not stored in DB) */
const BUILTIN_TROUBLESHOOT_PREVIEW: Command = {
  name: 'troubleshoot-preview',
  description: 'Troubleshoot preview server',
  prompt: [
    'The preview server appears to be down.',
    'Please troubleshoot the preview server:',
    '1. Check status: `/opt/moatless/bin/server preview status`',
    '2. Check logs: `/opt/moatless/bin/server preview logs`',
    '3. Try restarting: `/opt/moatless/bin/server preview restart`',
    '4. If it fails, check the config and fix the start command, then restart.',
  ].join('\n'),
  icon: 'Wrench',
  variant: 'default',
}

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
 * - Inject built-in commands based on workspace runtime state
 *
 * @dependencies
 * - CommandService - Get all defined commands
 * - WorkspaceRuntimeManager - Get workspace driver for git status
 * - TaskRepository - Get task data
 */
@injectable()
export class TaskCommandGateway {
  constructor(
    @inject(CommandService) private commandService: CommandService,
    @inject(WorkspaceRuntimeManager) private runtimeManager: WorkspaceRuntimeManager,
    @inject(TaskRepository) private taskRepository: TaskRepository,
  ) {}

  /**
   * Get available commands for a task
   *
   * Filters the full command list based on:
   * - Task status
   * - Last iteration status
   * - Git status (uncommitted changes, commits ahead/behind)
   * - Command conditions
   * - Workspace runtime state (built-in commands)
   *
   * @param taskId - Task ID to get commands for
   * @returns Filtered list of available commands
   * @throws TaskNotFoundError if task doesn't exist
   */
  async getAvailableCommands(taskId: string): Promise<Command[]> {
    // Get task data
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Get all commands
    const allCommands = await this.commandService.listCommands()

    // Get git status to check for uncommitted changes
    const handle = await this.runtimeManager.ensureWorkspace(taskId)
    const gitStatus = await handle.driver.getGitStatus()
    const workspaceHasChanges = gitStatus?.hasUncommittedChanges

    // Build command context
    const context = {
      task,
      ...(workspaceHasChanges !== undefined && { workspaceHasChanges }),
      ...(gitStatus !== undefined && { gitStatus }),
    }

    // Filter DB commands based on conditions
    const filtered = filterAvailableCommands(allCommands, context)

    // Add applicable built-in commands (pre-filtered by runtime state)
    const builtins = await this.getBuiltInCommands(taskId)
    return [...filtered, ...builtins]
  }

  /** Return built-in commands applicable based on workspace runtime state */
  private async getBuiltInCommands(taskId: string): Promise<Command[]> {
    const result: Command[] = []
    try {
      const snapshot = await this.runtimeManager.getSnapshot(taskId)
      const previewStatus = snapshot?.preview?.status
      if (!previewStatus || previewStatus === 'stopped' || previewStatus === 'failed') {
        result.push(BUILTIN_TROUBLESHOOT_PREVIEW)
      }
    } catch {
      // Non-fatal — skip built-in commands if snapshot unavailable
    }
    return result
  }
}
