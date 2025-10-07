import type { Command, GitStatus, Task, TaskIteration } from '@moatless/types'

type CommandConditions = NonNullable<Command['conditions']>

interface CommandAvailabilityContext {
  task?: Task
  lastIteration?: TaskIteration
  workspaceHasChanges?: boolean
  gitStatus?: GitStatus
}

// Debug logging flag - set to true to enable detailed command filtering logs
const DEBUG_COMMAND_FILTERING = false

function debugLog(message: string, data?: unknown) {
  if (DEBUG_COMMAND_FILTERING) {
    if (data !== undefined) {
      console.log(`[Command Filter] ${message}`, data)
    } else {
      console.log(`[Command Filter] ${message}`)
    }
  }
}

// Helper functions for cleaner condition checking
function checkTaskConditions(conditions: CommandConditions, task?: Task): boolean {
  // Note: Task 'running' status no longer exists - agent state is tracked separately
  // The requiresTaskNotRunning condition should be handled by checking agent state
  if (conditions.requiresTaskNotRunning) {
    // This condition now needs to be checked at a higher level with agent state
    debugLog('Task not running condition needs agent state check - assuming not running')
  }

  if (conditions.allowedTaskStatuses?.length && task) {
    const allowed = conditions.allowedTaskStatuses.includes(task.status)
    debugLog('Checking allowed task statuses', {
      allowed: conditions.allowedTaskStatuses,
      current: task.status,
      result: allowed,
    })
    return allowed
  }

  return true
}

function checkIterationConditions(
  conditions: CommandConditions,
  lastIteration?: TaskIteration,
): boolean {
  if (conditions.requiresLastIterationError) {
    const hasError = Boolean(
      lastIteration && (lastIteration.status === 'error' || lastIteration.status === 'failed'),
    )
    debugLog('Checking for iteration error', {
      required: true,
      iterationStatus: lastIteration?.status,
      result: hasError,
    })
    return hasError
  }

  if (conditions.requiresLastIterationSuccess) {
    const hasSuccess = Boolean(lastIteration && lastIteration.status === 'completed')
    debugLog('Checking for iteration success', {
      required: true,
      iterationStatus: lastIteration?.status,
      result: hasSuccess,
    })
    return hasSuccess
  }

  return true
}

function checkGitConditions(
  conditions: CommandConditions,
  workspaceHasChanges?: boolean,
  gitSyncStatus?: GitStatus,
): boolean {
  debugLog('Checking git conditions', {
    workspaceHasChanges,
    gitSyncStatus,
    conditions: {
      requiresUncommittedChanges: conditions.requiresUncommittedChanges,
      requiresCommittedChanges: conditions.requiresCommittedChanges,
      requiresAheadOfMain: conditions.requiresAheadOfMain,
      requiresBehindMain: conditions.requiresBehindMain,
      requiresDiverged: conditions.requiresDiverged,
      allowedGitSyncStatuses: conditions.allowedGitSyncStatuses,
    },
  })

  // List of git condition checkers - any one returning true means condition is satisfied (OR logic)
  const gitChecks = [
    () => {
      const result = Boolean(conditions.requiresUncommittedChanges && workspaceHasChanges)
      if (conditions.requiresUncommittedChanges) {
        debugLog('Uncommitted changes check', {
          required: true,
          hasChanges: workspaceHasChanges,
          result,
        })
      }
      return result
    },
    () => {
      const result = Boolean(
        conditions.requiresCommittedChanges && gitSyncStatus && gitSyncStatus.commitsAhead > 0,
      )
      if (conditions.requiresCommittedChanges) {
        debugLog('Committed changes check', {
          required: true,
          hasUncommitted: gitSyncStatus?.hasUncommittedChanges,
          commitsAhead: gitSyncStatus?.commitsAhead,
          result,
        })
      }
      return result
    },
    () => {
      const result = Boolean(conditions.requiresAheadOfMain && gitSyncStatus?.commitsAhead > 0)
      if (conditions.requiresAheadOfMain) {
        debugLog('Ahead of main check', {
          required: true,
          commitsAhead: gitSyncStatus?.commitsAhead,
          result,
        })
      }
      return result
    },
    () => {
      const result = Boolean(conditions.requiresBehindMain && gitSyncStatus?.commitsBehind > 0)
      if (conditions.requiresBehindMain) {
        debugLog('Behind main check', {
          required: true,
          commitsBehind: gitSyncStatus?.commitsBehind,
          result,
        })
      }
      return result
    },
    () => {
      const result = Boolean(conditions.requiresDiverged && gitSyncStatus?.status === 'diverged')
      if (conditions.requiresDiverged) {
        debugLog('Diverged check', {
          required: true,
          status: gitSyncStatus?.status,
          result,
        })
      }
      return result
    },
    () => {
      const result = Boolean(
        conditions.allowedGitSyncStatuses?.length &&
          gitSyncStatus &&
          conditions.allowedGitSyncStatuses.includes(gitSyncStatus.status),
      )
      if (conditions.allowedGitSyncStatuses?.length > 0) {
        debugLog('Allowed git statuses check', {
          allowed: conditions.allowedGitSyncStatuses,
          current: gitSyncStatus?.status,
          result,
        })
      }
      return result
    },
  ]

  const anyPassed = gitChecks.some((check) => check())
  debugLog('Git conditions result (OR logic)', { anyPassed })
  return anyPassed
}

function hasAnyGitConditions(conditions: CommandConditions): boolean {
  return Boolean(
    conditions.requiresUncommittedChanges ||
      conditions.requiresCommittedChanges ||
      conditions.requiresBehindMain ||
      conditions.requiresAheadOfMain ||
      conditions.requiresDiverged ||
      conditions.allowedGitSyncStatuses?.length > 0,
  )
}

export function isCommandAvailable(command: Command, context: CommandAvailabilityContext): boolean {
  debugLog(`Evaluating command: ${command.name}`)

  // Quick exits
  if (command.active === false) {
    debugLog(`Command ${command.name} is inactive`)
    return false
  }
  if (!command.conditions) {
    debugLog(`Command ${command.name} has no conditions - available`)
    return true
  }

  const conditions: CommandConditions = command.conditions
  const { task, lastIteration, workspaceHasChanges, gitStatus } = context
  const gitSyncStatus = gitStatus

  // All non-git conditions must pass (AND logic)
  const taskConditionsPassed = checkTaskConditions(conditions, task)
  if (!taskConditionsPassed) {
    debugLog(`Command ${command.name} failed task conditions`)
    return false
  }

  const iterationConditionsPassed = checkIterationConditions(conditions, lastIteration)
  if (!iterationConditionsPassed) {
    debugLog(`Command ${command.name} failed iteration conditions`)
    return false
  }

  // Git conditions use OR logic - at least one must pass if any are specified
  if (hasAnyGitConditions(conditions)) {
    const gitConditionsPassed = checkGitConditions(conditions, workspaceHasChanges, gitSyncStatus)
    debugLog(`Command ${command.name} git conditions result: ${gitConditionsPassed}`)
    return gitConditionsPassed
  }

  debugLog(`Command ${command.name} is available (no git conditions)`)
  return true
}

export function filterAvailableCommands(
  commands: Command[],
  context: CommandAvailabilityContext,
): Command[] {
  debugLog(`Filtering ${commands.length} commands`, { context })
  const available = commands.filter((command) => isCommandAvailable(command, context))
  debugLog(`Found ${available.length} available commands`, {
    availableCommands: available.map((c) => c.name),
  })
  return available
}
