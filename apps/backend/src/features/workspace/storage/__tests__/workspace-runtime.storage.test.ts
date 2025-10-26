import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StorageBackend } from '../../../../storage/storage-backend'
import { StorageSymbol } from '../../../../storage/storage-backend'
import { createWorkspaceSnapshot } from '../../types/workspace-runtime.types'
import { WorkspaceRuntimeStorage } from '../workspace-runtime.storage'

describe('WorkspaceRuntimeStorage', () => {
  let container: Container
  let storage: WorkspaceRuntimeStorage
  let mockBackend: StorageBackend

  beforeEach(() => {
    container = new Container()

    // Create mock storage backend
    mockBackend = {
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

    container.bind(StorageSymbol).toConstantValue(mockBackend)
    container.bind(WorkspaceRuntimeStorage).toSelf()

    storage = container.get(WorkspaceRuntimeStorage)
  })

  afterEach(() => {
    container.unbindAll()
    vi.clearAllMocks()
  })

  describe('save', () => {
    it('should save snapshot with correct key format', async () => {
      const snapshot = createWorkspaceSnapshot('task-123', 'repo-456', '/path/to/workspace')

      await storage.save(snapshot)

      expect(mockBackend.save).toHaveBeenCalledWith(
        'workspace-runtime/task-123.json',
        expect.objectContaining({
          taskId: 'task-123',
          repositoryId: 'repo-456',
          executionTarget: expect.objectContaining({
            workspacePath: '/path/to/workspace',
          }),
        }),
      )
    })

    it('should validate snapshot before saving', async () => {
      const invalidSnapshot = {
        taskId: 'task-123',
        // Missing required fields
      }

      await expect(storage.save(invalidSnapshot as never)).rejects.toThrow()
      expect(mockBackend.save).not.toHaveBeenCalled()
    })
  })

  describe('load', () => {
    it('should load and validate snapshot', async () => {
      const snapshot = createWorkspaceSnapshot('task-123', 'repo-456', '/path/to/workspace')
      ;(mockBackend.get as ReturnType<typeof vi.fn>).mockResolvedValue(snapshot)

      const loaded = await storage.load('task-123')

      expect(mockBackend.get).toHaveBeenCalledWith('workspace-runtime/task-123.json')
      expect(loaded).toEqual(snapshot)
    })

    it('should return null if snapshot does not exist', async () => {
      ;(mockBackend.get as ReturnType<typeof vi.fn>).mockResolvedValue(null)

      const loaded = await storage.load('task-123')

      expect(loaded).toBeNull()
    })

    it('should return null if snapshot is invalid', async () => {
      ;(mockBackend.get as ReturnType<typeof vi.fn>).mockResolvedValue({ invalid: 'data' })

      const loaded = await storage.load('task-123')

      expect(loaded).toBeNull()
    })
  })

  describe('update', () => {
    it('should update snapshot and bump config revision', async () => {
      const snapshot = createWorkspaceSnapshot('task-123', 'repo-456', '/path/to/workspace')
      ;(mockBackend.get as ReturnType<typeof vi.fn>).mockResolvedValue(snapshot)

      const success = await storage.update('task-123', (s) => ({
        ...s,
        preview: {
          ...s.preview,
          status: 'started',
          endpoint: 'http://localhost:3000',
        },
      }))

      expect(success).toBe(true)
      expect(mockBackend.save).toHaveBeenCalledWith(
        'workspace-runtime/task-123.json',
        expect.objectContaining({
          configRevision: '2',
          preview: expect.objectContaining({
            status: 'started',
            endpoint: 'http://localhost:3000',
          }),
        }),
      )
    })

    it('should return false if snapshot does not exist', async () => {
      ;(mockBackend.get as ReturnType<typeof vi.fn>).mockResolvedValue(null)

      const success = await storage.update('task-123', (s) => s)

      expect(success).toBe(false)
      expect(mockBackend.save).not.toHaveBeenCalled()
    })
  })

  describe('delete', () => {
    it('should delete snapshot with correct key', async () => {
      await storage.delete('task-123')

      expect(mockBackend.delete).toHaveBeenCalledWith('workspace-runtime/task-123.json')
    })
  })

  describe('list', () => {
    it('should list all valid snapshots', async () => {
      const snapshot1 = createWorkspaceSnapshot('task-1', 'repo-1', '/path/1')
      const snapshot2 = createWorkspaceSnapshot('task-2', 'repo-2', '/path/2')

      ;(mockBackend.list as ReturnType<typeof vi.fn>).mockResolvedValue([
        'workspace-runtime/task-1.json',
        'workspace-runtime/task-2.json',
      ])
      ;(mockBackend.get as ReturnType<typeof vi.fn>)
        .mockResolvedValueOnce(snapshot1)
        .mockResolvedValueOnce(snapshot2)

      const snapshots = await storage.list()

      expect(snapshots).toHaveLength(2)
      expect(snapshots[0]?.taskId).toBe('task-1')
      expect(snapshots[1]?.taskId).toBe('task-2')
    })

    it('should filter out invalid snapshots', async () => {
      const snapshot1 = createWorkspaceSnapshot('task-1', 'repo-1', '/path/1')

      ;(mockBackend.list as ReturnType<typeof vi.fn>).mockResolvedValue([
        'workspace-runtime/task-1.json',
        'workspace-runtime/task-2.json',
      ])
      ;(mockBackend.get as ReturnType<typeof vi.fn>)
        .mockResolvedValueOnce(snapshot1)
        .mockResolvedValueOnce({ invalid: 'data' })

      const snapshots = await storage.list()

      expect(snapshots).toHaveLength(1)
      expect(snapshots[0]?.taskId).toBe('task-1')
    })
  })

  describe('exists', () => {
    it('should check if snapshot exists', async () => {
      ;(mockBackend.exists as ReturnType<typeof vi.fn>).mockResolvedValue(true)

      const exists = await storage.exists('task-123')

      expect(mockBackend.exists).toHaveBeenCalledWith('workspace-runtime/task-123.json')
      expect(exists).toBe(true)
    })
  })
})
