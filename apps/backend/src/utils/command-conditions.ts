import type { Command, Task } from '@moatless/types'

type CommandConditions = NonNullable<Command['conditions']>

interface CommandAvailabilityContext {
  task?: Task
  workspaceHasChanges?: boolean
}

// Debug logging disabled - can be enabled by uncommenting and using logger
function debugLog(_message: string, _data?: unknown) {
  // const DEBUG_COMMAND_FILTERING = false
  // if (DEBUG_COMMAND_FILTERING) { logger.debug(_message, _data) }
}

// Helper functions for cleaner condition checking
function checkTaskConditions(conditions: CommandConditions, task?: Task): boolean {
  // Note: Task 'running' status no longer exists - agent state is tracked separately
  // The requiresTaskNotRunning condition should be handled by checking agent state
  if (conditions.requiresTaskNotRunning) {
    // This condition now needs to be checked at a higher level with agent state
    debugLog('Task not running condition needs agent state check - assuming not running')
  }

  if (conditions.allowedTaskStatuses && conditions.allowedTaskStatuses.length > 0 && task) {
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

function checkTurnConditions(conditions: CommandConditions, task?: Task): boolean {
  if (conditions.requiresLastIterationError) {
    const hasError = task?.status === 'error'
    debugLog('Checking for task error status', {
      required: true,
      taskStatus: task?.status,
      result: hasError,
    })
    return Boolean(hasError)
  }

  if (conditions.requiresLastIterationSuccess) {
    const hasSuccess = task?.status === 'open'
    debugLog('Checking for task open status (success)', {
      required: true,
      taskStatus: task?.status,
      result: hasSuccess,
    })
    return Boolean(hasSuccess)
  }

  return true
}

function checkWorkspaceConditions(
  conditions: CommandConditions,
  workspaceHasChanges?: boolean,
): boolean {
  if (conditions.requiresUncommittedChanges) {
    const result = Boolean(workspaceHasChanges)
    debugLog('Uncommitted changes check', {
      required: true,
      hasChanges: workspaceHasChanges,
      result,
    })
    return result
  }

  return true
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
  const { task, workspaceHasChanges } = context

  // All conditions must pass (AND logic)
  const taskConditionsPassed = checkTaskConditions(conditions, task)
  if (!taskConditionsPassed) {
    debugLog(`Command ${command.name} failed task conditions`)
    return false
  }

  const turnConditionsPassed = checkTurnConditions(conditions, task)
  if (!turnConditionsPassed) {
    debugLog(`Command ${command.name} failed turn conditions`)
    return false
  }

  const workspaceConditionsPassed = checkWorkspaceConditions(conditions, workspaceHasChanges)
  if (!workspaceConditionsPassed) {
    debugLog(`Command ${command.name} failed workspace conditions`)
    return false
  }

  debugLog(`Command ${command.name} is available`)
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
