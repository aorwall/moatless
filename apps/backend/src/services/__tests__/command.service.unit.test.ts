import type { Command, CreateCommandRequest, UpdateCommandRequest } from '@moatless/types'
import { CommandNotFoundError, ValidationError } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StorageBackend } from '../../storage/index.js'
import { CommandService } from '../command.service.js'

// Create mock storage
const createMockStorage = (): StorageBackend => ({
  save: vi.fn(),
  get: vi.fn(),
  delete: vi.fn(),
  list: vi.fn().mockResolvedValue([]),
  exists: vi.fn(),
  appendJsonl: vi.fn(),
  readJsonl: vi.fn().mockResolvedValue([]),
  saveTask: vi.fn(),
  getTask: vi.fn(),
  saveEvents: vi.fn(),
  getEvents: vi.fn(),
  saveWorkspace: vi.fn(),
  getWorkspace: vi.fn(),
  saveUser: vi.fn(),
  getUser: vi.fn(),
  listUsers: vi.fn().mockResolvedValue([]),
  saveFigmaImport: vi.fn(),
  getFigmaImport: vi.fn(),
})

describe('CommandService', () => {
  let service: CommandService
  let mockStorage: StorageBackend

  const getTestCommands = () => ({
    commands: {
      commit: {
        name: 'commit',
        description: 'Commit the changes to the repository',
        prompt: 'commit the relevant changes',
        conditions: {
          requiresUncommittedChanges: true,
          requiresTaskNotRunning: true,
        },
        icon: 'GitCommit',
        variant: 'outline',
        type: 'command',
      },
      continue: {
        name: 'continue',
        description: 'Continue after an error',
        prompt: 'continue with the task',
        conditions: {
          requiresLastIterationError: true,
          requiresTaskNotRunning: true,
        },
        icon: 'Play',
        variant: 'default',
        type: 'command',
      },
    },
  })

  beforeEach(() => {
    mockStorage = createMockStorage()
    service = new CommandService(mockStorage)
    vi.clearAllMocks()

    // Reset the mock to ensure clean state
    vi.mocked(mockStorage.get).mockReset()
    vi.mocked(mockStorage.save).mockReset()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('loadCommands', () => {
    it('should load existing commands from storage', async () => {
      const customCommands = {
        commands: {
          test: {
            name: 'test',
            description: 'Test command',
            prompt: 'run tests',
            conditions: {},
            type: 'command',
          },
        },
      }
      vi.mocked(mockStorage.get).mockResolvedValue(customCommands)

      const commands = await service.listCommands()

      expect(commands).toHaveLength(1)
      expect(commands[0]?.name).toBe('test')
    })

    it('should return default commands when none exist', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(null)

      const commands = await service.listCommands()

      // When data is null, it returns defaults without saving
      expect(mockStorage.save).not.toHaveBeenCalled()
      expect(commands.length).toBeGreaterThan(0)
      // Verify some default commands are present
      const commandNames = commands.map((c) => c.name)
      expect(commandNames).toContain('commit')
      expect(commandNames).toContain('continue')
      expect(commandNames).toContain('create-pr')
    })

    it('should handle storage errors gracefully', async () => {
      vi.mocked(mockStorage.get).mockRejectedValue(new Error('Storage error'))

      const commands = await service.listCommands()

      expect(mockStorage.save).toHaveBeenCalled()
      expect(commands.length).toBeGreaterThan(0)
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

    it('should create a new command successfully', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

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
      expect(mockStorage.save).toHaveBeenCalledWith(
        'commands',
        expect.objectContaining({
          commands: expect.objectContaining({
            commit: expect.any(Object),
            continue: expect.any(Object),
            deploy: expect.any(Object),
          }),
        }),
      )
    })

    it('should throw error if command name already exists', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

      const duplicateCommand = {
        ...newCommand,
        name: 'commit',
      }

      await expect(service.createCommand(duplicateCommand)).rejects.toThrow(ValidationError)
      await expect(service.createCommand(duplicateCommand)).rejects.toThrow(
        "Command with name 'commit' already exists",
      )
    })
  })

  describe('getCommand', () => {
    it('should return command when found', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

      const result = await service.getCommand('commit')

      expect(result).toMatchObject({
        name: 'commit',
        description: 'Commit the changes to the repository',
      })
    })

    it('should return null when command not found', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

      const result = await service.getCommand('non-existent')

      expect(result).toBeNull()
    })
  })

  describe('listCommands', () => {
    it('should return all commands as array', async () => {
      // Set up a fresh mock with exactly what we want
      const specificCommands = {
        commands: {
          test1: {
            name: 'test1',
            description: 'Test command 1',
            prompt: 'test1',
            conditions: {},
            type: 'command',
          },
          test2: {
            name: 'test2',
            description: 'Test command 2',
            prompt: 'test2',
            conditions: {},
            type: 'command',
          },
        },
      }
      vi.mocked(mockStorage.get).mockResolvedValueOnce(specificCommands)

      const result = await service.listCommands()

      expect(result).toBeInstanceOf(Array)
      expect(result).toHaveLength(2)
      expect(result.map((cmd: Command) => cmd.name)).toContain('test1')
      expect(result.map((cmd: Command) => cmd.name)).toContain('test2')
    })

    it('should return empty array when no commands exist', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue({ commands: {} })

      const result = await service.listCommands()

      expect(result).toEqual([])
    })
  })

  describe('updateCommand', () => {
    const updates: UpdateCommandRequest = {
      description: 'Updated description',
      prompt: 'updated prompt',
      active: false,
    }

    it('should update command successfully', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

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
      expect(mockStorage.save).toHaveBeenCalled()
    })

    it('should handle command name change', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

      const nameUpdate = {
        ...updates,
        name: 'commit-changes',
      }

      const result = await service.updateCommand('commit', nameUpdate)

      expect(result.name).toBe('commit-changes')
      expect(mockStorage.save).toHaveBeenCalledWith(
        'commands',
        expect.objectContaining({
          commands: expect.objectContaining({
            'commit-changes': expect.any(Object),
          }),
        }),
      )
      // Old name should be removed
      const savedData = vi.mocked(mockStorage.save).mock.calls[0]?.[1] as any
      expect(savedData.commands.commit).toBeUndefined()
    })

    it('should throw error when updating to existing name', async () => {
      // Mock needs to return the same value for multiple calls
      vi.mocked(mockStorage.get).mockImplementation(() => Promise.resolve(getTestCommands()))

      const nameUpdate = {
        name: 'continue',
      }

      await expect(service.updateCommand('commit', nameUpdate)).rejects.toThrow(ValidationError)
      await expect(service.updateCommand('commit', nameUpdate)).rejects.toThrow(
        "Command with name 'continue' already exists",
      )
    })

    it('should throw CommandNotFoundError when command not found', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

      await expect(service.updateCommand('non-existent', updates)).rejects.toThrow(
        CommandNotFoundError,
      )
    })
  })

  describe('deleteCommand', () => {
    it('should delete command successfully', async () => {
      // Use mockImplementation to ensure consistent return value
      vi.mocked(mockStorage.get).mockImplementation(() => Promise.resolve(getTestCommands()))

      await service.deleteCommand('commit')

      expect(mockStorage.save).toHaveBeenCalledWith(
        'commands',
        expect.objectContaining({
          commands: expect.not.objectContaining({
            commit: expect.any(Object),
          }),
        }),
      )
      // Continue command should still exist
      const savedData = vi.mocked(mockStorage.save).mock.calls[0]?.[1] as any
      expect(savedData.commands.continue).toBeDefined()
    })

    it('should throw CommandNotFoundError when command not found', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(getTestCommands())

      await expect(service.deleteCommand('non-existent')).rejects.toThrow(CommandNotFoundError)
    })
  })

  describe('default commands', () => {
    it('should include all expected default commands', async () => {
      vi.mocked(mockStorage.get).mockResolvedValue(null)

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
      vi.mocked(mockStorage.get).mockResolvedValue(null)

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
