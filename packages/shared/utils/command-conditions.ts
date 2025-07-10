import type { Command } from '../types/command.js'
import type { Task, TaskIteration } from '../types/task.js'

interface CommandAvailabilityContext {
  task?: Task
  lastIteration?: TaskIteration
  workspaceHasChanges?: boolean
}

export function isCommandAvailable(command: Command, context: CommandAvailabilityContext): boolean {
  // Check if command is active (defaults to true if not specified)
  if (command.active === false) {
    return false
  }

  if (!command.conditions) {
    return true // No conditions means always available
  }

  const { conditions } = command
  const { task, lastIteration, workspaceHasChanges } = context

  // Check task running status
  if (conditions.requiresTaskNotRunning && task?.status === 'running') {
    return false
  }

  // Check uncommitted changes
  if (conditions.requiresUncommittedChanges && !workspaceHasChanges) {
    return false
  }

  // Check last iteration error
  if (conditions.requiresLastIterationError) {
    if (!lastIteration || (lastIteration.status !== 'error' && lastIteration.status !== 'failed')) {
      return false
    }
  }

  // Check last iteration success
  if (conditions.requiresLastIterationSuccess) {
    if (!lastIteration || lastIteration.status !== 'completed') {
      return false
    }
  }

  // Check git sync status
  const gitSyncStatus = lastIteration?.gitSyncStatus
  if (gitSyncStatus) {
    // Check specific conditions
    if (conditions.requiresCommittedChanges && !gitSyncStatus.hasUncommittedChanges && gitSyncStatus.commitsAhead === 0) {
      return false
    }

    if (conditions.requiresAheadOfMain && gitSyncStatus.commitsAhead === 0) {
      return false
    }

    if (conditions.requiresBehindMain && gitSyncStatus.commitsBehind === 0) {
      return false
    }

    if (conditions.requiresDiverged && gitSyncStatus.status !== 'diverged') {
      return false
    }

    // Check allowed git sync statuses
    if (conditions.allowedGitSyncStatuses && conditions.allowedGitSyncStatuses.length > 0) {
      if (!conditions.allowedGitSyncStatuses.includes(gitSyncStatus.status)) {
        return false
      }
    }
  }

  // Check allowed task statuses
  if (conditions.allowedTaskStatuses && conditions.allowedTaskStatuses.length > 0 && task) {
    if (!conditions.allowedTaskStatuses.includes(task.status)) {
      return false
    }
  }

  return true
}

export function filterAvailableCommands(commands: Command[], context: CommandAvailabilityContext): Command[] {
  return commands.filter(command => isCommandAvailable(command, context))
}