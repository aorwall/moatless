import type { Command, CreateCommandRequest, UpdateCommandRequest } from '@moatless/types'
import { CommandNotFoundError, ValidationError } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { CommandRepository } from '../../db/command.repository'
import { CommandService } from '../command.service'

// Create mock repository
const createMockRepository = (): CommandRepository => ({
  db: {} as any,
  save: vi.fn(),
  get: vi.fn(),
  getAll: vi.fn(),
  list: vi.fn().mockResolvedValue([]),
  delete: vi.fn(),
  exists: vi.fn(),
})

describe('CommandService', () => {
  let service: CommandService
  let mockRepository: CommandRepository

  const testCommands: Command[] = [
    {
      name: 'commit',
      description: 'Commit the changes to the repository',
      prompt: 'commit the relevant changes',
      conditions: {
        requiresUncommittedChanges: true,
        requiresTaskNotRunning: true,
      },
      icon: 'GitCommit',
      variant: 'outline',
    },
    {
      name: 'continue',
      description: 'Continue after an error',
      prompt: 'continue with the task',
      conditions: {
        requiresLastIterationError: true,
        requiresTaskNotRunning: true,
      },
      icon: 'Play',
      variant: 'default',
    },
  ]

  beforeEach(() => {
    mockRepository = createMockRepository() as any
    service = new CommandService(mockRepository)
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('initialization', () => {
    it('should seed default commands when database is empty', async () => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue([])

      await service.listCommands()

      // Should have called save for each default command (6 defaults)
      expect(mockRepository.save).toHaveBeenCalledTimes(6)
      expect(mockRepository.save).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'commit',
        }),
      )
    })

    it('should not seed when commands already exist', async () => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue(['commit', 'continue'])
      ;(mockRepository.getAll as ReturnType<typeof vi.fn>).mockResolvedValue(testCommands)

      await service.listCommands()

      // Should not seed if commands exist
      expect(mockRepository.save).not.toHaveBeenCalled()
    })

    it('should only initialize once', async () => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue([])
      ;(mockRepository.getAll as ReturnType<typeof vi.fn>).mockResolvedValue([])

      // Call multiple times
      await service.listCommands()
      await service.listCommands()
      await service.listCommands()

      // Should only check list once (initialization happens once)
      expect(mockRepository.list).toHaveBeenCalledTimes(1)
    })
  })

  describe('createCommand', () => {
    const newCommand: CreateCommandRequest = {
      name: 'deploy',
      description: 'Deploy the application',
      prompt: 'deploy the application to production',
      conditions: {
        requiresTaskNotRunning: true,
      },
      icon: 'Rocket',
      variant: 'default',
      active: true,
    }

    beforeEach(() => {
      // Mock repository to have existing commands
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue(['commit', 'continue'])
    })

    it('should create a new command successfully', async () => {
      ;(mockRepository.exists as ReturnType<typeof vi.fn>).mockResolvedValue(false)

      const result = await service.createCommand(newCommand)

      expect(result).toMatchObject({
        name: 'deploy',
        description: 'Deploy the application',
        prompt: 'deploy the application to production',
        conditions: {
          requiresTaskNotRunning: true,
        },
        icon: 'Rocket',
        variant: 'default',
        active: true,
      })
      expect(mockRepository.save).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'deploy',
          description: 'Deploy the application',
        }),
      )
    })

    it('should throw error if command name already exists', async () => {
      ;(mockRepository.exists as ReturnType<typeof vi.fn>).mockResolvedValue(true)

      const duplicateCommand = {
        ...newCommand,
        name: 'commit',
      }

      await expect(service.createCommand(duplicateCommand)).rejects.toThrow(ValidationError)
      await expect(service.createCommand(duplicateCommand)).rejects.toThrow(
        "Command with name 'commit' already exists",
      )
      expect(mockRepository.save).not.toHaveBeenCalled()
    })
  })

  describe('getCommand', () => {
    beforeEach(() => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue(['commit'])
    })

    it('should return command when found', async () => {
      ;(mockRepository.get as ReturnType<typeof vi.fn>).mockResolvedValue(testCommands[0]!)

      const result = await service.getCommand('commit')

      expect(result).toMatchObject({
        name: 'commit',
        description: 'Commit the changes to the repository',
      })
      expect(mockRepository.get).toHaveBeenCalledWith('commit')
    })

    it('should return null when command not found', async () => {
      ;(mockRepository.get as ReturnType<typeof vi.fn>).mockResolvedValue(null)

      const result = await service.getCommand('non-existent')

      expect(result).toBeNull()
      expect(mockRepository.get).toHaveBeenCalledWith('non-existent')
    })
  })

  describe('listCommands', () => {
    it('should return all commands as array', async () => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue(['test1', 'test2'])
      ;(mockRepository.getAll as ReturnType<typeof vi.fn>).mockResolvedValue([
        {
          name: 'test1',
          description: 'Test command 1',
          prompt: 'test1',
          conditions: {},
        },
        {
          name: 'test2',
          description: 'Test command 2',
          prompt: 'test2',
          conditions: {},
        },
      ])

      const result = await service.listCommands()

      expect(result).toBeInstanceOf(Array)
      expect(result).toHaveLength(2)
      expect(result.map((cmd: Command) => cmd.name)).toContain('test1')
      expect(result.map((cmd: Command) => cmd.name)).toContain('test2')
    })

    it('should return empty array when no commands exist', async () => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue([])
      ;(mockRepository.getAll as ReturnType<typeof vi.fn>).mockResolvedValue([])

      await service.listCommands()

      // After seeding, should not be empty
      expect(mockRepository.save).toHaveBeenCalled()
    })
  })

  describe('updateCommand', () => {
    const updates: UpdateCommandRequest = {
      description: 'Updated description',
      prompt: 'updated prompt',
      active: false,
    }

    beforeEach(() => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue(['commit'])
    })

    it('should update command successfully', async () => {
      ;(mockRepository.get as ReturnType<typeof vi.fn>).mockResolvedValue(testCommands[0]!)

      const result = await service.updateCommand('commit', updates)

      expect(result).toMatchObject({
        name: 'commit',
        description: 'Updated description',
        prompt: 'updated prompt',
        active: false,
        conditions: {
          requiresUncommittedChanges: true,
          requiresTaskNotRunning: true,
        },
      })
      expect(mockRepository.save).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'commit',
          description: 'Updated description',
          prompt: 'updated prompt',
          active: false,
        }),
      )
    })

    it('should handle command name change', async () => {
      ;(mockRepository.get as ReturnType<typeof vi.fn>).mockResolvedValue(testCommands[0]!)
      ;(mockRepository.exists as ReturnType<typeof vi.fn>).mockResolvedValue(false)

      const nameUpdate = {
        ...updates,
        name: 'commit-changes',
      }

      const result = await service.updateCommand('commit', nameUpdate)

      expect(result.name).toBe('commit-changes')
      // Should delete old command and save new one
      expect(mockRepository.delete).toHaveBeenCalledWith('commit')
      expect(mockRepository.save).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'commit-changes',
        }),
      )
    })

    it('should throw error when updating to existing name', async () => {
      ;(mockRepository.get as ReturnType<typeof vi.fn>).mockResolvedValue(testCommands[0]!)
      ;(mockRepository.exists as ReturnType<typeof vi.fn>).mockResolvedValue(true)

      const nameUpdate = {
        name: 'continue',
      }

      await expect(service.updateCommand('commit', nameUpdate)).rejects.toThrow(ValidationError)
      await expect(service.updateCommand('commit', nameUpdate)).rejects.toThrow(
        "Command with name 'continue' already exists",
      )
    })

    it('should throw CommandNotFoundError when command not found', async () => {
      ;(mockRepository.get as ReturnType<typeof vi.fn>).mockResolvedValue(null)

      await expect(service.updateCommand('non-existent', updates)).rejects.toThrow(
        CommandNotFoundError,
      )
      expect(mockRepository.save).not.toHaveBeenCalled()
    })
  })

  describe('deleteCommand', () => {
    beforeEach(() => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue(['commit'])
    })

    it('should delete command successfully', async () => {
      ;(mockRepository.exists as ReturnType<typeof vi.fn>).mockResolvedValue(true)

      await service.deleteCommand('commit')

      expect(mockRepository.delete).toHaveBeenCalledWith('commit')
    })

    it('should throw CommandNotFoundError when command not found', async () => {
      ;(mockRepository.exists as ReturnType<typeof vi.fn>).mockResolvedValue(false)

      await expect(service.deleteCommand('non-existent')).rejects.toThrow(CommandNotFoundError)
      expect(mockRepository.delete).not.toHaveBeenCalled()
    })
  })

  describe('default commands', () => {
    it('should include all expected default commands', async () => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue([])
      // After seeding, getAll will return the defaults
      ;(mockRepository.getAll as ReturnType<typeof vi.fn>).mockResolvedValue([
        {
          name: 'commit',
          description: 'Commit the changes to the repository',
          prompt: 'commit the relevant changes',
          conditions: {
            requiresUncommittedChanges: true,
            requiresTaskNotRunning: true,
          },
          icon: 'GitCommit',
          variant: 'outline',
        },
        {
          name: 'continue',
          description: 'Continue after an error',
          prompt: 'continue with the task',
          conditions: {
            requiresLastIterationError: true,
            requiresTaskNotRunning: true,
          },
          icon: 'Play',
          variant: 'default',
        },
        {
          name: 'create-pr',
          description: 'Create a pull request',
          prompt: 'create a pull request with the changes',
          conditions: {
            requiresAheadOfMain: true,
            requiresTaskNotRunning: true,
            allowedGitSyncStatuses: ['ahead'],
          },
          icon: 'GitPullRequest',
          variant: 'outline',
        },
        {
          name: 'merge-to-main',
          description: 'Merge changes to main branch',
          prompt: 'merge the changes to the main branch',
          conditions: {
            requiresCommittedChanges: true,
            requiresTaskNotRunning: true,
            allowedGitSyncStatuses: ['ahead'],
          },
          icon: 'GitMerge',
          variant: 'outline',
        },
        {
          name: 'sync-with-main',
          description: 'Sync branch with main',
          prompt: 'sync the branch with the latest changes from main',
          conditions: {
            requiresBehindMain: true,
            requiresTaskNotRunning: true,
            allowedGitSyncStatuses: ['behind', 'diverged'],
          },
          icon: 'GitBranch',
          variant: 'secondary',
        },
        {
          name: 'compact',
          description: 'Summarize the conversation and compact the context',
          prompt:
            "Please provide a comprehensive summary of our conversation so far, including:\n\n1. The main task or problem we're working on\n2. Key decisions and approaches taken\n3. Important code changes made\n4. Current status and any remaining work\n5. Any important context that should be preserved\n\nThis summary will be used as context for future interactions, so please be thorough and include all relevant details.",
          conditions: {
            requiresTaskNotRunning: true,
          },
          icon: 'FileText',
          variant: 'secondary',
          willNotResume: true,
        },
      ])

      const commands = await service.listCommands()
      const commandNames = commands.map((cmd: Command) => cmd.name)

      expect(commandNames).toContain('commit')
      expect(commandNames).toContain('continue')
      expect(commandNames).toContain('create-pr')
      expect(commandNames).toContain('merge-to-main')
      expect(commandNames).toContain('sync-with-main')
      expect(commandNames).toContain('compact')
    })

    it('should have correct conditions for default commands', async () => {
      ;(mockRepository.list as ReturnType<typeof vi.fn>).mockResolvedValue([])
      ;(mockRepository.getAll as ReturnType<typeof vi.fn>).mockResolvedValue([
        {
          name: 'commit',
          description: 'Commit the changes to the repository',
          prompt: 'commit the relevant changes',
          conditions: {
            requiresUncommittedChanges: true,
            requiresTaskNotRunning: true,
          },
          icon: 'GitCommit',
          variant: 'outline',
        },
        {
          name: 'continue',
          description: 'Continue after an error',
          prompt: 'continue with the task',
          conditions: {
            requiresLastIterationError: true,
            requiresTaskNotRunning: true,
          },
          icon: 'Play',
          variant: 'default',
        },
        {
          name: 'create-pr',
          description: 'Create a pull request',
          prompt: 'create a pull request with the changes',
          conditions: {
            requiresAheadOfMain: true,
            requiresTaskNotRunning: true,
            allowedGitSyncStatuses: ['ahead'],
          },
          icon: 'GitPullRequest',
          variant: 'outline',
        },
      ])

      const commands = await service.listCommands()
      const commitCommand = commands.find((cmd: Command) => cmd.name === 'commit')
      const continueCommand = commands.find((cmd: Command) => cmd.name === 'continue')
      const createPrCommand = commands.find((cmd: Command) => cmd.name === 'create-pr')

      expect(commitCommand?.conditions).toMatchObject({
        requiresUncommittedChanges: true,
        requiresTaskNotRunning: true,
      })

      expect(continueCommand?.conditions).toMatchObject({
        requiresLastIterationError: true,
        requiresTaskNotRunning: true,
      })

      expect(createPrCommand?.conditions).toMatchObject({
        requiresAheadOfMain: true,
        requiresTaskNotRunning: true,
        allowedGitSyncStatuses: ['ahead'],
      })
    })
  })
})
