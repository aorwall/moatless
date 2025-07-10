/**
 * Command availability filtering logic
 * 
 * To enable debug logging:
 * 1. Set DEBUG_COMMAND_FILTERING = true below
 * 2. Open browser console to see detailed logs of command filtering decisions
 * 3. Logs will show which conditions pass/fail for each command
 */

import type { Command } from '../types/command.js'
import type { Task, TaskIteration } from '../types/task.js'

interface CommandAvailabilityContext {
  task?: Task
  lastIteration?: TaskIteration
  workspaceHasChanges?: boolean
}

// Debug logging flag - set to true to enable detailed command filtering logs
const DEBUG_COMMAND_FILTERING = false

function debugLog(message: string, data?: any) {
  if (DEBUG_COMMAND_FILTERING) {
    if (data !== undefined) {
      console.log(`[Command Filter] ${message}`, data)
    } else {
      console.log(`[Command Filter] ${message}`)
    }
  }
}

// Helper functions for cleaner condition checking
function checkTaskConditions(conditions: any, task?: any): boolean {
  if (conditions.requiresTaskNotRunning && task?.status === 'running') {
    debugLog('Task is running but command requires not running', { taskStatus: task.status })
    return false
  }
  
  if (conditions.allowedTaskStatuses?.length > 0 && task) {
    const allowed = conditions.allowedTaskStatuses.includes(task.status)
    debugLog('Checking allowed task statuses', { 
      allowed: conditions.allowedTaskStatuses, 
      current: task.status, 
      result: allowed 
    })
    return allowed
  }
  
  return true
}

function checkIterationConditions(conditions: any, lastIteration?: any): boolean {
  if (conditions.requiresLastIterationError) {
    const hasError = lastIteration && (lastIteration.status === 'error' || lastIteration.status === 'failed')
    debugLog('Checking for iteration error', { 
      required: true, 
      iterationStatus: lastIteration?.status, 
      result: hasError 
    })
    return hasError
  }
  
  if (conditions.requiresLastIterationSuccess) {
    const hasSuccess = lastIteration && lastIteration.status === 'completed'
    debugLog('Checking for iteration success', { 
      required: true, 
      iterationStatus: lastIteration?.status, 
      result: hasSuccess 
    })
    return hasSuccess
  }
  
  return true
}

function checkGitConditions(conditions: any, workspaceHasChanges?: boolean, gitSyncStatus?: any): boolean {
  debugLog('Checking git conditions', {
    workspaceHasChanges,
    gitSyncStatus,
    conditions: {
      requiresUncommittedChanges: conditions.requiresUncommittedChanges,
      requiresCommittedChanges: conditions.requiresCommittedChanges,
      requiresAheadOfMain: conditions.requiresAheadOfMain,
      requiresBehindMain: conditions.requiresBehindMain,
      requiresDiverged: conditions.requiresDiverged,
      allowedGitSyncStatuses: conditions.allowedGitSyncStatuses
    }
  })

  // List of git condition checkers - any one returning true means condition is satisfied (OR logic)
  const gitChecks = [
    () => {
      const result = conditions.requiresUncommittedChanges && workspaceHasChanges
      if (conditions.requiresUncommittedChanges) {
        debugLog('Uncommitted changes check', { required: true, hasChanges: workspaceHasChanges, result })
      }
      return result
    },
    () => {
      const result = conditions.requiresCommittedChanges && gitSyncStatus && (!gitSyncStatus.hasUncommittedChanges || gitSyncStatus.commitsAhead > 0)
      if (conditions.requiresCommittedChanges) {
        debugLog('Committed changes check', { 
          required: true, 
          hasUncommitted: gitSyncStatus?.hasUncommittedChanges,
          commitsAhead: gitSyncStatus?.commitsAhead,
          result 
        })
      }
      return result
    },
    () => {
      const result = conditions.requiresAheadOfMain && gitSyncStatus?.commitsAhead > 0
      if (conditions.requiresAheadOfMain) {
        debugLog('Ahead of main check', { required: true, commitsAhead: gitSyncStatus?.commitsAhead, result })
      }
      return result
    },
    () => {
      const result = conditions.requiresBehindMain && gitSyncStatus?.commitsBehind > 0
      if (conditions.requiresBehindMain) {
        debugLog('Behind main check', { required: true, commitsBehind: gitSyncStatus?.commitsBehind, result })
      }
      return result
    },
    () => {
      const result = conditions.requiresDiverged && gitSyncStatus?.status === 'diverged'
      if (conditions.requiresDiverged) {
        debugLog('Diverged check', { required: true, status: gitSyncStatus?.status, result })
      }
      return result
    },
    () => {
      const result = conditions.allowedGitSyncStatuses?.length > 0 && gitSyncStatus && conditions.allowedGitSyncStatuses.includes(gitSyncStatus.status)
      if (conditions.allowedGitSyncStatuses?.length > 0) {
        debugLog('Allowed git statuses check', { 
          allowed: conditions.allowedGitSyncStatuses,
          current: gitSyncStatus?.status,
          result 
        })
      }
      return result
    }
  ]
  
  const anyPassed = gitChecks.some(check => check())
  debugLog('Git conditions result (OR logic)', { anyPassed })
  return anyPassed
}

function hasAnyGitConditions(conditions: any): boolean {
  return !!(
    conditions.requiresUncommittedChanges ||
    conditions.requiresCommittedChanges ||
    conditions.requiresBehindMain ||
    conditions.requiresAheadOfMain ||
    conditions.requiresDiverged ||
    (conditions.allowedGitSyncStatuses?.length > 0)
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

  const { conditions } = command
  const { task, lastIteration, workspaceHasChanges } = context
  const gitSyncStatus = lastIteration?.gitSyncStatus

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

export function filterAvailableCommands(commands: Command[], context: CommandAvailabilityContext): Command[] {
  debugLog(`Filtering ${commands.length} commands`, { context })
  const available = commands.filter(command => isCommandAvailable(command, context))
  debugLog(`Found ${available.length} available commands`, { 
    availableCommands: available.map(c => c.name) 
  })
  return available
}