import { promises as fs } from 'node:fs'
import { EventType } from '@moatless/shared/types/event'
import type { Event, Task, Workspace } from '@moatless/shared/types/index'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FileStorage } from '../storage.js'

describe('FileStorage', () => {
  const testDir = '/tmp/test-storage'
  let storage: FileStorage

  beforeEach(async () => {
    storage = new FileStorage(testDir)
    // Ensure test directory exists
    await fs.mkdir(testDir, { recursive: true })
  })

  afterEach(async () => {
    // Clean up test directory
    await fs.rm(testDir, { recursive: true, force: true })
  })

  describe('Generic storage methods', () => {
    it('should save and retrieve data', async () => {
      const testData = { foo: 'bar', num: 42 }
      await storage.save('test-key', testData)

      const retrieved = await storage.get('test-key')
      expect(retrieved).toEqual(testData)
    })

    it('should return null for non-existent keys', async () => {
      const result = await storage.get('non-existent')
      expect(result).toBeNull()
    })

    it('should list all keys', async () => {
      await storage.save('key1', { data: 1 })
      await storage.save('key2', { data: 2 })
      await storage.save('nested/key3', { data: 3 })

      const keys = await storage.list()
      expect(keys).toContain('key1')
      expect(keys).toContain('key2')
      expect(keys).toContain('nested/key3')
    })

    it('should list keys with prefix', async () => {
      await storage.save('prefix/key1', { data: 1 })
      await storage.save('prefix/key2', { data: 2 })
      await storage.save('other/key3', { data: 3 })

      const keys = await storage.list('prefix/')
      expect(keys).toHaveLength(2)
      expect(keys).toContain('prefix/key1')
      expect(keys).toContain('prefix/key2')
      expect(keys).not.toContain('other/key3')
    })

    it('should delete keys', async () => {
      await storage.save('to-delete', { data: 'test' })
      expect(await storage.exists('to-delete')).toBe(true)

      await storage.delete('to-delete')
      expect(await storage.exists('to-delete')).toBe(false)
    })

    it('should check if key exists', async () => {
      expect(await storage.exists('non-existent')).toBe(false)

      await storage.save('existing', { data: 'test' })
      expect(await storage.exists('existing')).toBe(true)
    })
  })

  describe('Type-safe convenience methods', () => {
    it('should save and retrieve tasks', async () => {
      const task: Task = {
        taskId: 'test-123',
        createdAt: new Date(),
        lastMessageAt: new Date(),
        agentType: 'test-agent',
        repositoryId: 'env-123',
        status: 'open',
        numTurns: 0,
        totalCostUsd: 0,
        iterations: [],
      }

      await storage.saveTask(task)
      const retrieved = await storage.getTask('test-123')

      expect(retrieved).toBeTruthy()
      expect(retrieved?.taskId).toBe('test-123')
      expect(retrieved?.agentType).toBe('test-agent')
    })

    it('should save and retrieve repositories', async () => {
      const repo = {
        id: 'repo-123',
        name: 'Test Repository',
        path: '/test/path',
        useGitWorktrees: false,
        codeExecution: { type: 'local' as const },
        variables: {},
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      await storage.saveRepository(repo)
      const retrieved = await storage.getRepository('repo-123')

      expect(retrieved).toBeTruthy()
      expect(retrieved?.id).toBe('repo-123')
      expect(retrieved?.name).toBe('Test Repository')
    })

    it('should save and retrieve workspaces', async () => {
      const workspace: Workspace = {
        id: 'ws-123',
        type: 'local',
        repositoryId: 'env-123',
        status: 'ready',
        createdAt: new Date(),
        path: '/tmp/workspace',
      }

      await storage.saveWorkspace(workspace)
      const retrieved = await storage.getWorkspace('ws-123')

      expect(retrieved).toBeTruthy()
      expect(retrieved?.id).toBe('ws-123')
      expect(retrieved?.type).toBe('local')
    })

    it('should save and retrieve events', async () => {
      const events: Event[] = [
        {
          eventId: 'evt-1',
          type: EventType.START,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          agentType: 'test-agent',
          payload: { message: 'Starting' },
        },
        {
          eventId: 'evt-2',
          type: EventType.MESSAGE,
          taskId: 'task-123',
          timestamp: new Date(),
          isError: false,
          agentType: 'test-agent',
          payload: { content: 'Hello' },
        },
      ]

      await storage.saveEvents('task-123', events)
      const retrieved = await storage.getEvents('task-123')

      expect(retrieved).toHaveLength(2)
      expect(retrieved[0].eventId).toBe('evt-1')
      expect(retrieved[1].eventId).toBe('evt-2')
    })

    it('should return empty array for non-existent events', async () => {
      const events = await storage.getEvents('non-existent-task')
      expect(events).toEqual([])
    })
  })

  describe('Date serialization', () => {
    it('should correctly serialize and deserialize dates', async () => {
      const now = new Date()
      const data = {
        createdAt: now,
        metadata: {
          timestamp: now,
        },
      }

      await storage.save('date-test', data)
      const retrieved = await storage.get<typeof data>('date-test')

      expect(retrieved?.createdAt).toBeInstanceOf(Date)
      expect(retrieved?.createdAt.toISOString()).toBe(now.toISOString())
      expect(retrieved?.metadata.timestamp).toBeInstanceOf(Date)
    })
  })

  describe('Error handling', () => {
    it('should handle file system errors gracefully', async () => {
      // Mock fs.readFile to throw an error
      const spy = vi.spyOn(fs, 'readFile').mockRejectedValueOnce(new Error('ENOENT'))

      const result = await storage.get('error-key')
      expect(result).toBeNull()

      spy.mockRestore()
    })

    it('should throw on delete if file cannot be removed', async () => {
      const spy = vi.spyOn(fs, 'unlink').mockRejectedValueOnce(new Error('EPERM'))

      await expect(storage.delete('protected-key')).rejects.toThrow('EPERM')

      spy.mockRestore()
    })
  })
})
