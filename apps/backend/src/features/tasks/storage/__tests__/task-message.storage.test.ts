import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StorageBackend } from '../../../../storage/storage-backend'
import { StorageSymbol } from '../../../../storage/storage-backend'
import { MessageStorage } from '../message.storage.ts'

describe('TaskMessageStorage', () => {
  let container: Container
  let taskMessageStorage: MessageStorage
  let mockStorage: StorageBackend

  beforeEach(() => {
    container = new Container()

    // Mock storage with JSONL methods
    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      list: vi.fn(),
      appendJsonl: vi.fn(),
      readJsonl: vi.fn(),
      saveTask: vi.fn(),
      getTask: vi.fn(),
      saveWorkspace: vi.fn(),
      getWorkspace: vi.fn(),
    }

    // Bind mocks
    container.bind(StorageSymbol).toConstantValue(mockStorage)
    container.bind(MessageStorage).toSelf()

    // Resolve adapter
    taskMessageStorage = container.get(MessageStorage)
  })

  afterEach(() => {
    container.unbindAll()
    vi.clearAllMocks()
  })

  describe('read', () => {
    it('should read messages from JSONL log', async () => {
      const mockMessages = [
        {
          uuid: 'msg-1',
          type: 'user',
          timestamp: '2025-01-01T00:00:00Z',
          message: { role: 'user', content: 'Hello' },
        },
        {
          uuid: 'msg-2',
          type: 'assistant',
          timestamp: '2025-01-01T00:00:01Z',
          message: { role: 'assistant', content: [{ type: 'text', text: 'Hi!' }] },
        },
      ]

      ;(mockStorage.readJsonl as any).mockResolvedValue(mockMessages)

      const result = await taskMessageStorage.read('task-123')

      expect(mockStorage.readJsonl).toHaveBeenCalledWith('messages/task-123')
      expect(result).toEqual(mockMessages)
    })

    it('should return empty array when file does not exist', async () => {
      ;(mockStorage.readJsonl as any).mockRejectedValue(new Error('File not found'))

      const result = await taskMessageStorage.read('task-nonexistent')

      expect(result).toEqual([])
    })

    it('should return empty array when read fails', async () => {
      ;(mockStorage.readJsonl as any).mockRejectedValue(new Error('Read error'))

      const result = await taskMessageStorage.read('task-error')

      expect(result).toEqual([])
    })
  })

  describe('append', () => {
    it('should append message to JSONL log', async () => {
      const message = {
        uuid: 'msg-3',
        type: 'user' as const,
        timestamp: '2025-01-01T00:00:02Z',
        userType: 'user' as const,
        message: {
          role: 'user' as const,
          content: [{ type: 'text' as const, text: 'New message' }],
        },
      }

      await taskMessageStorage.append('task-456', message)

      expect(mockStorage.appendJsonl).toHaveBeenCalledWith('messages/task-456', message)
    })
  })

  describe('delete', () => {
    it('should delete message log', async () => {
      await taskMessageStorage.delete('task-789')

      expect(mockStorage.delete).toHaveBeenCalledWith('messages/task-789')
    })
  })

  describe('exists', () => {
    it('should check if message log exists', async () => {
      ;(mockStorage.exists as any).mockResolvedValue(true)

      const result = await taskMessageStorage.exists('task-123')

      expect(mockStorage.exists).toHaveBeenCalledWith('messages/task-123')
      expect(result).toBe(true)
    })

    it('should return false when log does not exist', async () => {
      ;(mockStorage.exists as any).mockResolvedValue(false)

      const result = await taskMessageStorage.exists('nonexistent')

      expect(result).toBe(false)
    })
  })

  describe('key generation', () => {
    it('should use correct key format for all operations', async () => {
      const taskId = 'test-task-id'

      await taskMessageStorage.append(taskId, {
        uuid: 'test',
        type: 'user' as const,
        timestamp: '2025-01-01T00:00:00Z',
        userType: 'user' as const,
        message: { role: 'user' as const, content: [] },
      })

      await taskMessageStorage.read(taskId)
      await taskMessageStorage.exists(taskId)
      await taskMessageStorage.delete(taskId)

      // All operations should use the same key format
      expect(mockStorage.appendJsonl).toHaveBeenCalledWith(
        'messages/test-task-id',
        expect.anything(),
      )
      expect(mockStorage.readJsonl).toHaveBeenCalledWith('messages/test-task-id')
      expect(mockStorage.exists).toHaveBeenCalledWith('messages/test-task-id')
      expect(mockStorage.delete).toHaveBeenCalledWith('messages/test-task-id')
    })
  })
})
