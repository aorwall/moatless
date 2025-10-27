import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Storage } from '../../../../infrastructure/storage/storage'
import { InfrastructureStorageSymbol } from '../../../../infrastructure/storage/storage'
import { TaskMetadataStorage } from '../task-metadata.storage'

describe('TaskMetadataStorage', () => {
  let container: Container
  let taskMetadataStorage: TaskMetadataStorage
  let mockStorage: Storage

  beforeEach(() => {
    container = new Container()

    // Mock storage with minimal interface
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      list: vi.fn(),
    }

    // Bind mocks
    container.bind(InfrastructureStorageSymbol).toConstantValue(mockStorage)
    container.bind(TaskMetadataStorage).toSelf()

    // Resolve adapter
    taskMetadataStorage = container.get(TaskMetadataStorage)
  })

  afterEach(() => {
    container.unbindAll()
    vi.clearAllMocks()
  })

  describe('save', () => {
    it('should save task with correct key structure', async () => {
      const task = {
        taskId: 'task-123',
        status: 'open' as const,
        agentType: 'claude-code',
        repositoryId: 'repo-1',
        iterations: [],
        numTurns: 0,
        totalCostUsd: 0,
        createdAt: new Date(),
        lastMessageAt: new Date(),
      }

      await taskMetadataStorage.save(task)

      expect(mockStorage.save).toHaveBeenCalledWith('tasks/task-123', task)
    })
  })

  describe('get', () => {
    it('should retrieve task by ID', async () => {
      const mockTask = {
        taskId: 'task-456',
        status: 'open',
        agentType: 'claude-code',
      }

      ;(mockStorage.get as any).mockResolvedValue(mockTask)

      const result = await taskMetadataStorage.get('task-456')

      expect(mockStorage.get).toHaveBeenCalledWith('tasks/task-456')
      expect(result).toEqual(mockTask)
    })

    it('should return null when task does not exist', async () => {
      ;(mockStorage.get as any).mockResolvedValue(null)

      const result = await taskMetadataStorage.get('nonexistent')

      expect(result).toBeNull()
    })
  })

  describe('delete', () => {
    it('should delete task and associated events', async () => {
      await taskMetadataStorage.delete('task-789')

      expect(mockStorage.delete).toHaveBeenCalledWith('tasks/task-789')
      expect(mockStorage.delete).toHaveBeenCalledWith('events/task-789')
    })
  })

  describe('exists', () => {
    it('should check task existence', async () => {
      ;(mockStorage.exists as any).mockResolvedValue(true)

      const result = await taskMetadataStorage.exists('task-123')

      expect(mockStorage.exists).toHaveBeenCalledWith('tasks/task-123')
      expect(result).toBe(true)
    })

    it('should return false when task does not exist', async () => {
      ;(mockStorage.exists as any).mockResolvedValue(false)

      const result = await taskMetadataStorage.exists('nonexistent')

      expect(result).toBe(false)
    })
  })

  describe('listTaskKeys', () => {
    it('should list all task keys with correct prefix', async () => {
      const mockKeys = ['tasks/task-1', 'tasks/task-2', 'tasks/task-3']
      ;(mockStorage.list as any).mockResolvedValue(mockKeys)

      const result = await taskMetadataStorage.listTaskKeys()

      expect(mockStorage.list).toHaveBeenCalledWith('tasks/')
      expect(result).toEqual(mockKeys)
    })

    it('should return empty array when no tasks exist', async () => {
      ;(mockStorage.list as any).mockResolvedValue([])

      const result = await taskMetadataStorage.listTaskKeys()

      expect(result).toEqual([])
    })
  })
})
