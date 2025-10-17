import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { UserSession } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { SessionContext } from '../../services/access-control/index.js'
import { AuthorizedFileStorage } from '../authorized-file-storage.js'

describe('AuthorizedFileStorage', () => {
  const testDir = '/tmp/test-authorized-storage'
  let storage: AuthorizedFileStorage

  const testSession: UserSession = {
    id: 'session-123',
    provider: 'github',
    userId: '456',
    userLogin: 'testuser',
    userEmail: 'test@example.com',
    userName: 'Test User',
    avatarUrl: 'https://avatars.githubusercontent.com/u/456',
    accessToken: 'gho_testtoken',
    tokenType: 'bearer',
    scope: 'user:email',
    createdAt: new Date(),
    expiresAt: new Date(Date.now() + 86400000),
    lastAccessedAt: new Date(),
  }

  beforeEach(async () => {
    storage = new AuthorizedFileStorage(testDir)
    await fs.mkdir(testDir, { recursive: true })
  })

  afterEach(async () => {
    await fs.rm(testDir, { recursive: true, force: true })
  })

  describe('User-scoped operations', () => {
    it('should save data with user scope prefix', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-123', { title: 'Test Task' })

        // Verify file is saved in user-scoped path
        const expectedPath = path.join(testDir, 'users/456/tasks/task-123.json')
        const content = await fs.readFile(expectedPath, 'utf-8')
        const data = JSON.parse(content)

        expect(data.title).toBe('Test Task')
      })
    })

    it('should retrieve data from user-scoped path', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-123', { title: 'Test Task' })
        const retrieved = await storage.get('tasks/task-123')

        expect(retrieved).toEqual({ title: 'Test Task' })
      })
    })

    it('should list keys with user scope', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-1', { title: 'Task 1' })
        await storage.save('tasks/task-2', { title: 'Task 2' })
        await storage.save('repositories/repo-1', { name: 'Repo 1' })

        const taskKeys = await storage.list('tasks/')

        // Should return keys without the user scope prefix
        expect(taskKeys).toHaveLength(2)
        expect(taskKeys).toContain('tasks/task-1')
        expect(taskKeys).toContain('tasks/task-2')
        expect(taskKeys).not.toContain('repositories/repo-1')

        // Verify files are actually in user-scoped directories
        const task1Path = path.join(testDir, 'users/456/tasks/task-1.json')
        const task1Exists = await fs
          .access(task1Path)
          .then(() => true)
          .catch(() => false)
        expect(task1Exists).toBe(true)
      })
    })

    it('should delete data from user-scoped path', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-123', { title: 'Test Task' })
        expect(await storage.exists('tasks/task-123')).toBe(true)

        await storage.delete('tasks/task-123')
        expect(await storage.exists('tasks/task-123')).toBe(false)
      })
    })

    it('should check existence in user-scoped path', async () => {
      await SessionContext.run(testSession, async () => {
        expect(await storage.exists('tasks/non-existent')).toBe(false)

        await storage.save('tasks/task-123', { title: 'Test Task' })
        expect(await storage.exists('tasks/task-123')).toBe(true)
      })
    })
  })

  describe('Sessions exception - no user scoping', () => {
    it('should save sessions at root level without user scope', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('sessions/session-abc', { userId: 789 })

        // Verify file is saved at root level, not in user-scoped directory
        const expectedPath = path.join(testDir, 'sessions/session-abc.json')
        const content = await fs.readFile(expectedPath, 'utf-8')
        const data = JSON.parse(content)

        expect(data.userId).toBe(789)

        // Verify it's NOT in user-scoped directory
        const userScopedPath = path.join(testDir, 'users/456/sessions/session-abc.json')
        const userScopedExists = await fs
          .access(userScopedPath)
          .then(() => true)
          .catch(() => false)
        expect(userScopedExists).toBe(false)
      })
    })

    it('should list sessions from root level without user scope', async () => {
      await SessionContext.run(testSession, async () => {
        // Save sessions at root level
        await storage.save('sessions/session-1', { userId: 100 })
        await storage.save('sessions/session-2', { userId: 200 })

        const sessionKeys = await storage.list('sessions/')

        expect(sessionKeys).toHaveLength(2)
        expect(sessionKeys).toContain('sessions/session-1')
        expect(sessionKeys).toContain('sessions/session-2')
      })
    })
  })

  describe('User isolation', () => {
    const user1Session: UserSession = {
      id: 'session-user1',
      provider: 'github',
      userId: '100',
      userLogin: 'user1',
      userEmail: 'user1@example.com',
      userName: 'User One',
      avatarUrl: 'https://avatars.githubusercontent.com/u/100',
      accessToken: 'gho_user1token',
      tokenType: 'bearer',
      scope: 'user:email',
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 86400000),
      lastAccessedAt: new Date(),
    }

    const user2Session: UserSession = {
      id: 'session-user2',
      provider: 'github',
      userId: '200',
      userLogin: 'user2',
      userEmail: 'user2@example.com',
      userName: 'User Two',
      avatarUrl: 'https://avatars.githubusercontent.com/u/200',
      accessToken: 'gho_user2token',
      tokenType: 'bearer',
      scope: 'user:email',
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 86400000),
      lastAccessedAt: new Date(),
    }

    it('should isolate data between different users', async () => {
      // User 1 saves data
      await SessionContext.run(user1Session, async () => {
        await storage.save('tasks/task-1', { title: 'User 1 Task' })
      })

      // User 2 saves data with same key
      await SessionContext.run(user2Session, async () => {
        await storage.save('tasks/task-1', { title: 'User 2 Task' })
      })

      // User 1 should only see their data
      await SessionContext.run(user1Session, async () => {
        const data = await storage.get('tasks/task-1')
        expect(data).toEqual({ title: 'User 1 Task' })

        const keys = await storage.list('tasks/')
        expect(keys).toHaveLength(1)
      })

      // User 2 should only see their data
      await SessionContext.run(user2Session, async () => {
        const data = await storage.get('tasks/task-1')
        expect(data).toEqual({ title: 'User 2 Task' })

        const keys = await storage.list('tasks/')
        expect(keys).toHaveLength(1)
      })
    })
  })

  describe('Authorization checks', () => {
    it('should throw error when saving without session', async () => {
      await expect(storage.save('tasks/task-123', { title: 'Test' })).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should throw error when getting without session', async () => {
      await expect(storage.get('tasks/task-123')).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should throw error when listing without session', async () => {
      await expect(storage.list('tasks/')).rejects.toThrow('Unauthorized: No user session found')
    })

    it('should throw error when deleting without session', async () => {
      await expect(storage.delete('tasks/task-123')).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should throw error when checking existence without session', async () => {
      await expect(storage.exists('tasks/task-123')).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should allow session operations without session (bootstrap case)', async () => {
      // Sessions can be accessed without a session context (needed for auth middleware)
      await storage.save('sessions/session-123', { userId: 456 })
      const retrieved = await storage.get('sessions/session-123')
      expect(retrieved).toEqual({ userId: 456 })

      const keys = await storage.list('sessions/')
      expect(keys).toContain('sessions/session-123')
    })
  })

  describe('Type-safe convenience methods', () => {
    it('should save and retrieve repositories with user scoping', async () => {
      await SessionContext.run(testSession, async () => {
        const repo = {
          id: 'repo-123',
          name: 'Test Repository',
          path: '/test/path',
          useGitWorktrees: false,
          cloneOnNewTask: false,
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

        // Verify file is in user-scoped path
        const expectedPath = path.join(testDir, 'users/456/repositories/repo-123.json')
        const exists = await fs
          .access(expectedPath)
          .then(() => true)
          .catch(() => false)
        expect(exists).toBe(true)
      })
    })
  })
})
