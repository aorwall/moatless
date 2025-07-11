import type { Command } from '@moatless/shared/types/command'
import type { Task, TaskIteration } from '@moatless/shared/types/task'
import { describe, expect, it } from 'vitest'
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

    it('should return false for inactive commands', () => {
      const command: Command = {
        name: 'test',
        description: 'Test command',
        prompt: 'test prompt',
        active: false,
      }

      expect(isCommandAvailable(command, {})).toBe(false)
    })

    it('should return true for active commands', () => {
      const command: Command = {
        name: 'test',
        description: 'Test command',
        prompt: 'test prompt',
        active: true,
      }

      expect(isCommandAvailable(command, {})).toBe(true)
    })

    describe('task status conditions', () => {
      it('should respect requiresTaskNotRunning condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresTaskNotRunning: true,
          },
        }

        const runningTask: Task = {
          taskId: 'test-task',
          status: 'running',
          agentType: 'test',
          repositoryId: 'test-repo',
          createdAt: new Date(),
          lastMessageAt: new Date(),
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        }

        const openTask: Task = {
          ...runningTask,
          status: 'open',
        }

        expect(isCommandAvailable(command, { task: runningTask })).toBe(false)
        expect(isCommandAvailable(command, { task: openTask })).toBe(true)
      })

      it('should respect allowedTaskStatuses condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            allowedTaskStatuses: ['open', 'error'],
          },
        }

        const openTask: Task = {
          taskId: 'test-task',
          status: 'open',
          agentType: 'test',
          repositoryId: 'test-repo',
          createdAt: new Date(),
          lastMessageAt: new Date(),
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        }

        const runningTask: Task = {
          ...openTask,
          status: 'running',
        }

        const errorTask: Task = {
          ...openTask,
          status: 'error',
        }

        expect(isCommandAvailable(command, { task: openTask })).toBe(true)
        expect(isCommandAvailable(command, { task: errorTask })).toBe(true)
        expect(isCommandAvailable(command, { task: runningTask })).toBe(false)
      })
    })

    describe('iteration conditions', () => {
      it('should respect requiresLastIterationError condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresLastIterationError: true,
          },
        }

        const errorIteration: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'error',
          messageCount: 1,
          costUsd: 0,
        }

        const completedIteration: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed',
          messageCount: 1,
          costUsd: 0,
        }

        const failedIteration: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'failed',
          messageCount: 1,
          costUsd: 0,
        }

        expect(isCommandAvailable(command, { lastIteration: errorIteration })).toBe(true)
        expect(isCommandAvailable(command, { lastIteration: failedIteration })).toBe(true)
        expect(isCommandAvailable(command, { lastIteration: completedIteration })).toBe(false)
        expect(isCommandAvailable(command, {})).toBe(false)
      })

      it('should respect requiresLastIterationSuccess condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresLastIterationSuccess: true,
          },
        }

        const completedIteration: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed',
          messageCount: 1,
          costUsd: 0,
        }

        const errorIteration: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'error',
          messageCount: 1,
          costUsd: 0,
        }

        expect(isCommandAvailable(command, { lastIteration: completedIteration })).toBe(true)
        expect(isCommandAvailable(command, { lastIteration: errorIteration })).toBe(false)
        expect(isCommandAvailable(command, {})).toBe(false)
      })
    })

    describe('git conditions', () => {
      it('should respect requiresUncommittedChanges condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresUncommittedChanges: true,
          },
        }

        expect(isCommandAvailable(command, { workspaceHasChanges: true })).toBe(true)
        expect(isCommandAvailable(command, { workspaceHasChanges: false })).toBe(false)
        expect(isCommandAvailable(command, {})).toBe(false)
      })

      it('should respect requiresCommittedChanges condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresCommittedChanges: true,
          },
        }

        const iterationWithCommits: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed',
          messageCount: 1,
          costUsd: 0,
          gitSyncStatus: {
            status: 'ahead',
            commitsAhead: 2,
            commitsBehind: 0,
            hasUncommittedChanges: false,
            currentBranch: 'feature-branch',
            baseBranch: 'main',
            originExists: true,
          },
        }

        const iterationWithoutCommits: TaskIteration = {
          ...iterationWithCommits,
          gitSyncStatus: {
            ...iterationWithCommits.gitSyncStatus!,
            commitsAhead: 0,
            hasUncommittedChanges: false,
          },
        }

        expect(isCommandAvailable(command, { lastIteration: iterationWithCommits })).toBe(true)
        expect(isCommandAvailable(command, { lastIteration: iterationWithoutCommits })).toBe(false)
        expect(isCommandAvailable(command, {})).toBe(false)
      })

      it('should respect requiresAheadOfMain condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresAheadOfMain: true,
          },
        }

        const iterationAhead: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed',
          messageCount: 1,
          costUsd: 0,
          gitSyncStatus: {
            status: 'ahead',
            commitsAhead: 2,
            commitsBehind: 0,
            hasUncommittedChanges: false,
            currentBranch: 'feature-branch',
            baseBranch: 'main',
            originExists: true,
          },
        }

        const iterationSync: TaskIteration = {
          ...iterationAhead,
          gitSyncStatus: {
            ...iterationAhead.gitSyncStatus!,
            status: 'synced',
            commitsAhead: 0,
          },
        }

        expect(isCommandAvailable(command, { lastIteration: iterationAhead })).toBe(true)
        expect(isCommandAvailable(command, { lastIteration: iterationSync })).toBe(false)
      })

      it('should respect allowedGitSyncStatuses condition', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            allowedGitSyncStatuses: ['ahead', 'diverged'],
          },
        }

        const iterationAhead: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed',
          messageCount: 1,
          costUsd: 0,
          gitSyncStatus: {
            status: 'ahead',
            commitsAhead: 2,
            commitsBehind: 0,
            hasUncommittedChanges: false,
            currentBranch: 'feature-branch',
            baseBranch: 'main',
            originExists: true,
          },
        }

        const iterationDiverged: TaskIteration = {
          ...iterationAhead,
          gitSyncStatus: {
            ...iterationAhead.gitSyncStatus!,
            status: 'diverged',
            commitsBehind: 1,
          },
        }

        const iterationSynced: TaskIteration = {
          ...iterationAhead,
          gitSyncStatus: {
            ...iterationAhead.gitSyncStatus!,
            status: 'synced',
            commitsAhead: 0,
          },
        }

        expect(isCommandAvailable(command, { lastIteration: iterationAhead })).toBe(true)
        expect(isCommandAvailable(command, { lastIteration: iterationDiverged })).toBe(true)
        expect(isCommandAvailable(command, { lastIteration: iterationSynced })).toBe(false)
      })

      it('should use OR logic for git conditions', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresUncommittedChanges: true,
            requiresAheadOfMain: true,
          },
        }

        const iterationWithUncommitted: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed',
          messageCount: 1,
          costUsd: 0,
          gitSyncStatus: {
            status: 'uncommitted_changes',
            commitsAhead: 0,
            commitsBehind: 0,
            hasUncommittedChanges: true,
            currentBranch: 'feature-branch',
            baseBranch: 'main',
            originExists: true,
          },
        }

        const iterationAhead: TaskIteration = {
          ...iterationWithUncommitted,
          gitSyncStatus: {
            ...iterationWithUncommitted.gitSyncStatus!,
            status: 'ahead',
            commitsAhead: 2,
            hasUncommittedChanges: false,
          },
        }

        const iterationClean: TaskIteration = {
          ...iterationWithUncommitted,
          gitSyncStatus: {
            ...iterationWithUncommitted.gitSyncStatus!,
            status: 'synced',
            commitsAhead: 0,
            hasUncommittedChanges: false,
          },
        }

        // Should pass if workspace has uncommitted changes
        expect(
          isCommandAvailable(command, {
            lastIteration: iterationClean,
            workspaceHasChanges: true,
          }),
        ).toBe(true)

        // Should pass if ahead of main
        expect(
          isCommandAvailable(command, {
            lastIteration: iterationAhead,
            workspaceHasChanges: false,
          }),
        ).toBe(true)

        // Should fail if neither condition is met
        expect(
          isCommandAvailable(command, {
            lastIteration: iterationClean,
            workspaceHasChanges: false,
          }),
        ).toBe(false)
      })
    })

    describe('combined conditions', () => {
      it('should require all non-git conditions to pass (AND logic)', () => {
        const command: Command = {
          name: 'test',
          description: 'Test command',
          prompt: 'test prompt',
          conditions: {
            requiresTaskNotRunning: true,
            requiresLastIterationSuccess: true,
            requiresUncommittedChanges: true,
          },
        }

        const task: Task = {
          taskId: 'test-task',
          status: 'open',
          agentType: 'test',
          repositoryId: 'test-repo',
          createdAt: new Date(),
          lastMessageAt: new Date(),
          numTurns: 0,
          totalCostUsd: 0,
          iterations: [],
        }

        const completedIteration: TaskIteration = {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed',
          messageCount: 1,
          costUsd: 0,
        }

        // All conditions met
        expect(
          isCommandAvailable(command, {
            task,
            lastIteration: completedIteration,
            workspaceHasChanges: true,
          }),
        ).toBe(true)

        // Task running - should fail
        expect(
          isCommandAvailable(command, {
            task: { ...task, status: 'running' },
            lastIteration: completedIteration,
            workspaceHasChanges: true,
          }),
        ).toBe(false)

        // Last iteration failed - should fail
        expect(
          isCommandAvailable(command, {
            task,
            lastIteration: { ...completedIteration, status: 'error' },
            workspaceHasChanges: true,
          }),
        ).toBe(false)

        // No uncommitted changes - should fail
        expect(
          isCommandAvailable(command, {
            task,
            lastIteration: completedIteration,
            workspaceHasChanges: false,
          }),
        ).toBe(false)
      })
    })
  })

  describe('filterAvailableCommands', () => {
    const commands: Command[] = [
      {
        name: 'always-available',
        description: 'Always available command',
        prompt: 'always available',
      },
      {
        name: 'commit',
        description: 'Commit changes',
        prompt: 'commit changes',
        conditions: {
          requiresUncommittedChanges: true,
        },
      },
      {
        name: 'retry',
        description: 'Retry last action',
        prompt: 'retry',
        conditions: {
          requiresLastIterationError: true,
        },
      },
      {
        name: 'inactive',
        description: 'Inactive command',
        prompt: 'inactive',
        active: false,
      },
    ]

    it('should filter commands based on context', () => {
      const context = {
        workspaceHasChanges: true,
        lastIteration: {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'error' as const,
          messageCount: 1,
          costUsd: 0,
        },
      }

      const available = filterAvailableCommands(commands, context)

      expect(available).toHaveLength(3)
      expect(available.map((c) => c.name)).toEqual(['always-available', 'commit', 'retry'])
    })

    it('should return only unconditional commands when no context matches', () => {
      const context = {
        workspaceHasChanges: false,
        lastIteration: {
          iterationNumber: 1,
          startTime: new Date(),
          status: 'completed' as const,
          messageCount: 1,
          costUsd: 0,
        },
      }

      const available = filterAvailableCommands(commands, context)

      expect(available).toHaveLength(1)
      expect(available[0].name).toBe('always-available')
    })

    it('should handle empty commands array', () => {
      const available = filterAvailableCommands([], {})
      expect(available).toHaveLength(0)
    })

    it('should handle empty context', () => {
      const available = filterAvailableCommands(commands, {})

      expect(available).toHaveLength(1)
      expect(available[0].name).toBe('always-available')
    })
  })
})
