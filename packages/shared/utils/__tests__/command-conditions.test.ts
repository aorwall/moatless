import { describe, expect, it } from 'vitest'
import type { Command } from '../../types/command.js'
import type { Task, TaskIteration } from '../../types/task.js'
import { filterAvailableCommands, isCommandAvailable } from '../command-conditions.js'

describe('Command Conditions', () => {
  describe('isCommandAvailable', () => {
    it('should return true for commands without conditions', () => {
      const command: Command = {
        name: 'test',
        description: 'Test command',
        prompt: 'test prompt',
      }

      expect(isCommandAvailable(command, {})).toBe(true)
    })

    it('should check active flag', () => {
      const activeCommand: Command = {
        name: 'test',
        description: 'Test command',
        prompt: 'test prompt',
        active: true,
      }

      const inactiveCommand: Command = {
        name: 'test',
        description: 'Test command',
        prompt: 'test prompt',
        active: false,
      }

      const undefinedActiveCommand: Command = {
        name: 'test',
        description: 'Test command',
        prompt: 'test prompt',
        // active is undefined, should default to true
      }

      expect(isCommandAvailable(activeCommand, {})).toBe(true)
      expect(isCommandAvailable(inactiveCommand, {})).toBe(false)
      expect(isCommandAvailable(undefinedActiveCommand, {})).toBe(true)
    })

    it('should check requiresUncommittedChanges condition', () => {
      const command: Command = {
        name: 'commit',
        description: 'Commit changes',
        prompt: 'commit changes',
        conditions: {
          requiresUncommittedChanges: true,
        },
      }

      expect(isCommandAvailable(command, { workspaceHasChanges: true })).toBe(true)
      expect(isCommandAvailable(command, { workspaceHasChanges: false })).toBe(false)
      expect(isCommandAvailable(command, {})).toBe(false)
    })

    it('should check requiresTaskNotRunning condition', () => {
      const command: Command = {
        name: 'test',
        description: 'Test',
        prompt: 'test',
        conditions: {
          requiresTaskNotRunning: true,
        },
      }

      const runningTask: Task = { status: 'running' } as Task
      const idleTask: Task = { status: 'open' } as Task

      expect(isCommandAvailable(command, { task: runningTask })).toBe(false)
      expect(isCommandAvailable(command, { task: idleTask })).toBe(true)
      expect(isCommandAvailable(command, {})).toBe(true)
    })

    it('should check requiresLastIterationError condition', () => {
      const command: Command = {
        name: 'continue',
        description: 'Continue',
        prompt: 'continue',
        conditions: {
          requiresLastIterationError: true,
        },
      }

      const errorIteration: TaskIteration = { status: 'error' } as TaskIteration
      const failedIteration: TaskIteration = { status: 'failed' } as TaskIteration
      const successIteration: TaskIteration = { status: 'completed' } as TaskIteration

      expect(isCommandAvailable(command, { lastIteration: errorIteration })).toBe(true)
      expect(isCommandAvailable(command, { lastIteration: failedIteration })).toBe(true)
      expect(isCommandAvailable(command, { lastIteration: successIteration })).toBe(false)
      expect(isCommandAvailable(command, {})).toBe(false)
    })

    it('should check requiresLastIterationSuccess condition', () => {
      const command: Command = {
        name: 'deploy',
        description: 'Deploy',
        prompt: 'deploy',
        conditions: {
          requiresLastIterationSuccess: true,
        },
      }

      const successIteration: TaskIteration = { status: 'completed' } as TaskIteration
      const errorIteration: TaskIteration = { status: 'error' } as TaskIteration

      expect(isCommandAvailable(command, { lastIteration: successIteration })).toBe(true)
      expect(isCommandAvailable(command, { lastIteration: errorIteration })).toBe(false)
      expect(isCommandAvailable(command, {})).toBe(false)
    })

    it('should check git sync status conditions', () => {
      const command: Command = {
        name: 'create-pr',
        description: 'Create PR',
        prompt: 'create pr',
        conditions: {
          requiresAheadOfMain: true,
        },
      }

      const aheadIteration: TaskIteration = {
        gitSyncStatus: {
          status: 'ahead',
          hasUncommittedChanges: false,
          commitsAhead: 2,
          commitsBehind: 0,
          currentBranch: 'feature',
          baseBranch: 'main',
        },
      } as TaskIteration

      const syncedIteration: TaskIteration = {
        gitSyncStatus: {
          status: 'synced',
          hasUncommittedChanges: false,
          commitsAhead: 0,
          commitsBehind: 0,
          currentBranch: 'main',
          baseBranch: 'main',
        },
      } as TaskIteration

      expect(isCommandAvailable(command, { lastIteration: aheadIteration })).toBe(true)
      expect(isCommandAvailable(command, { lastIteration: syncedIteration })).toBe(false)
    })

    it('should check requiresBehindMain condition', () => {
      const command: Command = {
        name: 'sync',
        description: 'Sync',
        prompt: 'sync',
        conditions: {
          requiresBehindMain: true,
        },
      }

      const behindIteration: TaskIteration = {
        gitSyncStatus: {
          status: 'behind',
          hasUncommittedChanges: false,
          commitsAhead: 0,
          commitsBehind: 3,
          currentBranch: 'feature',
          baseBranch: 'main',
        },
      } as TaskIteration

      const aheadIteration: TaskIteration = {
        gitSyncStatus: {
          status: 'ahead',
          hasUncommittedChanges: false,
          commitsAhead: 2,
          commitsBehind: 0,
          currentBranch: 'feature',
          baseBranch: 'main',
        },
      } as TaskIteration

      expect(isCommandAvailable(command, { lastIteration: behindIteration })).toBe(true)
      expect(isCommandAvailable(command, { lastIteration: aheadIteration })).toBe(false)
    })

    it('should check allowedGitSyncStatuses condition', () => {
      const command: Command = {
        name: 'sync',
        description: 'Sync',
        prompt: 'sync',
        conditions: {
          allowedGitSyncStatuses: ['behind', 'diverged'],
        },
      }

      const behindIteration: TaskIteration = {
        gitSyncStatus: {
          status: 'behind',
          hasUncommittedChanges: false,
          commitsAhead: 0,
          commitsBehind: 3,
          currentBranch: 'feature',
          baseBranch: 'main',
        },
      } as TaskIteration

      const divergedIteration: TaskIteration = {
        gitSyncStatus: {
          status: 'diverged',
          hasUncommittedChanges: false,
          commitsAhead: 2,
          commitsBehind: 3,
          currentBranch: 'feature',
          baseBranch: 'main',
        },
      } as TaskIteration

      const aheadIteration: TaskIteration = {
        gitSyncStatus: {
          status: 'ahead',
          hasUncommittedChanges: false,
          commitsAhead: 2,
          commitsBehind: 0,
          currentBranch: 'feature',
          baseBranch: 'main',
        },
      } as TaskIteration

      expect(isCommandAvailable(command, { lastIteration: behindIteration })).toBe(true)
      expect(isCommandAvailable(command, { lastIteration: divergedIteration })).toBe(true)
      expect(isCommandAvailable(command, { lastIteration: aheadIteration })).toBe(false)
    })

    it('should check allowedTaskStatuses condition', () => {
      const command: Command = {
        name: 'close',
        description: 'Close',
        prompt: 'close',
        conditions: {
          allowedTaskStatuses: ['open', 'error'],
        },
      }

      const openTask: Task = { status: 'open' } as Task
      const errorTask: Task = { status: 'error' } as Task
      const runningTask: Task = { status: 'running' } as Task

      expect(isCommandAvailable(command, { task: openTask })).toBe(true)
      expect(isCommandAvailable(command, { task: errorTask })).toBe(true)
      expect(isCommandAvailable(command, { task: runningTask })).toBe(false)
    })

    it('should check multiple conditions together', () => {
      const command: Command = {
        name: 'merge-to-main',
        description: 'Merge to main',
        prompt: 'merge to main',
        conditions: {
          requiresTaskNotRunning: true,
          requiresAheadOfMain: true,
          allowedGitSyncStatuses: ['ahead'],
        },
      }

      const validContext = {
        task: { status: 'open' } as Task,
        lastIteration: {
          gitSyncStatus: {
            status: 'ahead' as const,
            hasUncommittedChanges: false,
            commitsAhead: 2,
            commitsBehind: 0,
            currentBranch: 'feature',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      const runningTaskContext = {
        ...validContext,
        task: { status: 'running' } as Task,
      }

      const wrongGitStatusContext = {
        ...validContext,
        lastIteration: {
          gitSyncStatus: {
            ...validContext.lastIteration.gitSyncStatus,
            status: 'behind' as const,
            commitsAhead: 0,
            commitsBehind: 2,
          },
        } as TaskIteration,
      }

      expect(isCommandAvailable(command, validContext)).toBe(true)
      expect(isCommandAvailable(command, runningTaskContext)).toBe(false)
      expect(isCommandAvailable(command, wrongGitStatusContext)).toBe(false)
    })
  })

  describe('filterAvailableCommands', () => {
    it('should filter commands based on context', () => {
      const commands: Command[] = [
        {
          name: 'commit',
          description: 'Commit',
          prompt: 'commit',
          conditions: {
            requiresUncommittedChanges: true,
          },
        },
        {
          name: 'continue',
          description: 'Continue',
          prompt: 'continue',
          conditions: {
            requiresLastIterationError: true,
          },
        },
        {
          name: 'no-conditions',
          description: 'No conditions',
          prompt: 'no conditions',
        },
      ]

      const context = {
        workspaceHasChanges: true,
        lastIteration: { status: 'completed' } as TaskIteration,
      }

      const available = filterAvailableCommands(commands, context)
      expect(available).toHaveLength(2)
      expect(available.map((c: Command) => c.name)).toEqual(['commit', 'no-conditions'])
    })

    it('should return empty array when no commands match', () => {
      const commands: Command[] = [
        {
          name: 'commit',
          description: 'Commit',
          prompt: 'commit',
          conditions: {
            requiresUncommittedChanges: true,
          },
        },
        {
          name: 'continue',
          description: 'Continue',
          prompt: 'continue',
          conditions: {
            requiresLastIterationError: true,
          },
        },
      ]

      const context = {
        workspaceHasChanges: false,
        lastIteration: { status: 'completed' } as TaskIteration,
      }

      const available = filterAvailableCommands(commands, context)
      expect(available).toHaveLength(0)
    })

    it('should filter out inactive commands even if conditions would pass', () => {
      const commands: Command[] = [
        {
          name: 'active-command',
          description: 'Active command',
          prompt: 'active',
          active: true,
          conditions: {
            requiresUncommittedChanges: true,
          },
        },
        {
          name: 'inactive-command',
          description: 'Inactive command',
          prompt: 'inactive',
          active: false,
          conditions: {
            requiresUncommittedChanges: true,
          },
        },
      ]

      const context = {
        workspaceHasChanges: true, // Conditions would normally pass
      }

      const available = filterAvailableCommands(commands, context)
      expect(available).toHaveLength(1)
      expect(available[0].name).toBe('active-command')
    })

    it('should return command when only one git status condition is set and satisfied', () => {
      // Test that a command with just requiresUncommittedChanges works
      const commitCommand: Command = {
        name: 'commit',
        description: 'Commit changes',
        prompt: 'commit',
        conditions: {
          requiresUncommittedChanges: true,
          // Note: NOT setting other git conditions like requiresAheadOfMain, etc.
        },
      }

      // Context with uncommitted changes but various git sync statuses
      const contextWithChanges = {
        workspaceHasChanges: true,
        lastIteration: {
          gitSyncStatus: {
            status: 'synced' as const,
            hasUncommittedChanges: true,
            commitsAhead: 0,
            commitsBehind: 0,
            currentBranch: 'main',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      const contextBehindWithChanges = {
        workspaceHasChanges: true,
        lastIteration: {
          gitSyncStatus: {
            status: 'behind' as const,
            hasUncommittedChanges: true,
            commitsAhead: 0,
            commitsBehind: 3,
            currentBranch: 'feature',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      const contextWithoutChanges = {
        workspaceHasChanges: false,
        lastIteration: {
          gitSyncStatus: {
            status: 'synced' as const,
            hasUncommittedChanges: false,
            commitsAhead: 0,
            commitsBehind: 0,
            currentBranch: 'main',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      // Command should be available when there are uncommitted changes
      expect(isCommandAvailable(commitCommand, contextWithChanges)).toBe(true)
      expect(isCommandAvailable(commitCommand, contextBehindWithChanges)).toBe(true)

      // Command should NOT be available when there are no uncommitted changes
      expect(isCommandAvailable(commitCommand, contextWithoutChanges)).toBe(false)

      // Test with requiresAheadOfMain only
      const prCommand: Command = {
        name: 'create-pr',
        description: 'Create PR',
        prompt: 'create pr',
        conditions: {
          requiresAheadOfMain: true,
          // Not setting other conditions
        },
      }

      const contextAhead = {
        lastIteration: {
          gitSyncStatus: {
            status: 'ahead' as const,
            hasUncommittedChanges: false,
            commitsAhead: 2,
            commitsBehind: 0,
            currentBranch: 'feature',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      const contextNotAhead = {
        lastIteration: {
          gitSyncStatus: {
            status: 'synced' as const,
            hasUncommittedChanges: false,
            commitsAhead: 0,
            commitsBehind: 0,
            currentBranch: 'main',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      // Command should be available when branch is ahead
      expect(isCommandAvailable(prCommand, contextAhead)).toBe(true)

      // Command should NOT be available when branch is not ahead
      expect(isCommandAvailable(prCommand, contextNotAhead)).toBe(false)
    })

    it('should use OR logic for multiple git status conditions', () => {
      // Command with multiple git conditions - ANY one should make it available
      const flexibleCommand: Command = {
        name: 'flexible',
        description: 'Flexible command',
        prompt: 'flexible command',
        conditions: {
          requiresUncommittedChanges: true,
          requiresAheadOfMain: true,
          requiresBehindMain: true,
        },
      }

      // Context 1: Has uncommitted changes but not ahead/behind
      const contextUncommitted = {
        workspaceHasChanges: true,
        lastIteration: {
          gitSyncStatus: {
            status: 'synced' as const,
            hasUncommittedChanges: true,
            commitsAhead: 0,
            commitsBehind: 0,
            currentBranch: 'main',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      // Context 2: Is ahead but no uncommitted changes
      const contextAhead = {
        workspaceHasChanges: false,
        lastIteration: {
          gitSyncStatus: {
            status: 'ahead' as const,
            hasUncommittedChanges: false,
            commitsAhead: 2,
            commitsBehind: 0,
            currentBranch: 'feature',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      // Context 3: Is behind but no uncommitted changes
      const contextBehind = {
        workspaceHasChanges: false,
        lastIteration: {
          gitSyncStatus: {
            status: 'behind' as const,
            hasUncommittedChanges: false,
            commitsAhead: 0,
            commitsBehind: 3,
            currentBranch: 'feature',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      // Context 4: None of the conditions are met
      const contextNone = {
        workspaceHasChanges: false,
        lastIteration: {
          gitSyncStatus: {
            status: 'synced' as const,
            hasUncommittedChanges: false,
            commitsAhead: 0,
            commitsBehind: 0,
            currentBranch: 'main',
            baseBranch: 'main',
          },
        } as TaskIteration,
      }

      // Command should be available when ANY git condition is satisfied
      expect(isCommandAvailable(flexibleCommand, contextUncommitted)).toBe(true) // uncommitted changes
      expect(isCommandAvailable(flexibleCommand, contextAhead)).toBe(true) // ahead of main
      expect(isCommandAvailable(flexibleCommand, contextBehind)).toBe(true) // behind main

      // Command should NOT be available when NO git conditions are satisfied
      expect(isCommandAvailable(flexibleCommand, contextNone)).toBe(false)
    })

    it('should still require non-git conditions to be satisfied along with git OR conditions', () => {
      // Command with git conditions (OR) AND task condition (AND)
      const command: Command = {
        name: 'mixed',
        description: 'Mixed conditions',
        prompt: 'mixed',
        conditions: {
          requiresUncommittedChanges: true,
          requiresAheadOfMain: true,
          requiresTaskNotRunning: true, // This is a task condition, not git
        },
      }

      const runningTask = { status: 'running' } as Task
      const openTask = { status: 'open' } as Task

      const contextWithChanges = {
        workspaceHasChanges: true,
        task: openTask,
        lastIteration: {
          gitSyncStatus: {
            status: 'synced' as const,
            hasUncommittedChanges: true,
            commitsAhead: 0,
            commitsBehind: 0,
          },
        } as TaskIteration,
      }

      const contextWithChangesButRunning = {
        workspaceHasChanges: true,
        task: runningTask,
        lastIteration: {
          gitSyncStatus: {
            status: 'synced' as const,
            hasUncommittedChanges: true,
            commitsAhead: 0,
            commitsBehind: 0,
          },
        } as TaskIteration,
      }

      // Should be available when git condition is satisfied AND task condition is satisfied
      expect(isCommandAvailable(command, contextWithChanges)).toBe(true)

      // Should NOT be available when git condition is satisfied but task condition fails
      expect(isCommandAvailable(command, contextWithChangesButRunning)).toBe(false)
    })
  })
})
