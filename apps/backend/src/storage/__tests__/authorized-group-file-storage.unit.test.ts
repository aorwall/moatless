import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { UserSession } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { SessionContext } from '../../services/access-control/index.js'
import { AuthorizedGroupFileStorage } from '../authorized-group-file-storage.js'

describe('AuthorizedGroupFileStorage', () => {
  const testDir = '/tmp/test-authorized-group-storage'
  let storage: AuthorizedGroupFileStorage

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
    groupId: 'team-frontend',
    createdAt: new Date(),
    expiresAt: new Date(Date.now() + 86400000),
    lastAccessedAt: new Date(),
  }

  beforeEach(async () => {
    storage = new AuthorizedGroupFileStorage(testDir)
    await fs.mkdir(testDir, { recursive: true })
  })

  afterEach(async () => {
    await fs.rm(testDir, { recursive: true, force: true })
  })

  describe('Group-scoped operations', () => {
    it('should save data with group scope prefix', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-123', { title: 'Test Task' })

        // Verify file is saved in group-scoped path
        const expectedPath = path.join(testDir, 'groups/team-frontend/tasks/task-123.json')
        const content = await fs.readFile(expectedPath, 'utf-8')
        const data = JSON.parse(content)

        expect(data.title).toBe('Test Task')
      })
    })

    it('should retrieve data from group-scoped path', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-123', { title: 'Test Task' })
        const retrieved = await storage.get('tasks/task-123')

        expect(retrieved).toEqual({ title: 'Test Task' })
      })
    })

    it('should list keys with group scope', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-1', { title: 'Task 1' })
        await storage.save('tasks/task-2', { title: 'Task 2' })
        await storage.save('repositories/repo-1', { name: 'Repo 1' })

        const taskKeys = await storage.list('tasks/')

        // Should return keys without the group scope prefix
        expect(taskKeys).toHaveLength(2)
        expect(taskKeys).toContain('tasks/task-1')
        expect(taskKeys).toContain('tasks/task-2')
        expect(taskKeys).not.toContain('repositories/repo-1')

        // Verify files are actually in group-scoped directories
        const task1Path = path.join(testDir, 'groups/team-frontend/tasks/task-1.json')
        const task1Exists = await fs
          .access(task1Path)
          .then(() => true)
          .catch(() => false)
        expect(task1Exists).toBe(true)
      })
    })

    it('should delete data from group-scoped path', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('tasks/task-123', { title: 'Test Task' })
        expect(await storage.exists('tasks/task-123')).toBe(true)

        await storage.delete('tasks/task-123')
        expect(await storage.exists('tasks/task-123')).toBe(false)
      })
    })

    it('should check existence in group-scoped path', async () => {
      await SessionContext.run(testSession, async () => {
        expect(await storage.exists('tasks/non-existent')).toBe(false)

        await storage.save('tasks/task-123', { title: 'Test Task' })
        expect(await storage.exists('tasks/task-123')).toBe(true)
      })
    })
  })

  describe('Exceptions - no group scoping', () => {
    it('should save sessions at root level without group scope', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('sessions/session-abc', { userId: 789 })

        // Verify file is saved at root level, not in group-scoped directory
        const expectedPath = path.join(testDir, 'sessions/session-abc.json')
        const content = await fs.readFile(expectedPath, 'utf-8')
        const data = JSON.parse(content)

        expect(data.userId).toBe(789)

        // Verify it's NOT in group-scoped directory
        const groupScopedPath = path.join(testDir, 'groups/team-frontend/sessions/session-abc.json')
        const groupScopedExists = await fs
          .access(groupScopedPath)
          .then(() => true)
          .catch(() => false)
        expect(groupScopedExists).toBe(false)
      })
    })

    it('should save users at root level without group scope', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('users/user-123', { username: 'john' })

        // Verify file is saved at root level, not in group-scoped directory
        const expectedPath = path.join(testDir, 'users/user-123.json')
        const content = await fs.readFile(expectedPath, 'utf-8')
        const data = JSON.parse(content)

        expect(data.username).toBe('john')

        // Verify it's NOT in group-scoped directory
        const groupScopedPath = path.join(testDir, 'groups/team-frontend/users/user-123.json')
        const groupScopedExists = await fs
          .access(groupScopedPath)
          .then(() => true)
          .catch(() => false)
        expect(groupScopedExists).toBe(false)
      })
    })

    it('should list sessions from root level without group scope', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('sessions/session-1', { userId: 100 })
        await storage.save('sessions/session-2', { userId: 200 })

        const sessionKeys = await storage.list('sessions/')

        expect(sessionKeys).toHaveLength(2)
        expect(sessionKeys).toContain('sessions/session-1')
        expect(sessionKeys).toContain('sessions/session-2')
      })
    })

    it('should list users from root level without group scope', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('users/user-1', { username: 'alice' })
        await storage.save('users/user-2', { username: 'bob' })

        const userKeys = await storage.list('users/')

        expect(userKeys).toHaveLength(2)
        expect(userKeys).toContain('users/user-1')
        expect(userKeys).toContain('users/user-2')
      })
    })
  })

  describe('Group isolation', () => {
    const group1Session: UserSession = {
      id: 'session-group1',
      provider: 'github',
      userId: '100',
      userLogin: 'user1',
      userEmail: 'user1@example.com',
      userName: 'User One',
      avatarUrl: 'https://avatars.githubusercontent.com/u/100',
      accessToken: 'gho_user1token',
      tokenType: 'bearer',
      scope: 'user:email',
      groupId: 'team-frontend',
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 86400000),
      lastAccessedAt: new Date(),
    }

    const group2Session: UserSession = {
      id: 'session-group2',
      provider: 'github',
      userId: '200',
      userLogin: 'user2',
      userEmail: 'user2@example.com',
      userName: 'User Two',
      avatarUrl: 'https://avatars.githubusercontent.com/u/200',
      accessToken: 'gho_user2token',
      tokenType: 'bearer',
      scope: 'user:email',
      groupId: 'team-backend',
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 86400000),
      lastAccessedAt: new Date(),
    }

    it('should isolate data between different groups', async () => {
      // Group 1 saves data
      await SessionContext.run(group1Session, async () => {
        await storage.save('tasks/task-1', { title: 'Frontend Task' })
      })

      // Group 2 saves data with same key
      await SessionContext.run(group2Session, async () => {
        await storage.save('tasks/task-1', { title: 'Backend Task' })
      })

      // Group 1 should only see their data
      await SessionContext.run(group1Session, async () => {
        const data = await storage.get('tasks/task-1')
        expect(data).toEqual({ title: 'Frontend Task' })

        const keys = await storage.list('tasks/')
        expect(keys).toHaveLength(1)
      })

      // Group 2 should only see their data
      await SessionContext.run(group2Session, async () => {
        const data = await storage.get('tasks/task-1')
        expect(data).toEqual({ title: 'Backend Task' })

        const keys = await storage.list('tasks/')
        expect(keys).toHaveLength(1)
      })
    })
  })

  describe('Authorization checks', () => {
    const sessionWithoutGroup: UserSession = {
      id: 'session-no-group',
      provider: 'github',
      userId: '789',
      userLogin: 'nogroup',
      userEmail: 'nogroup@example.com',
      userName: 'No Group User',
      avatarUrl: 'https://avatars.githubusercontent.com/u/789',
      accessToken: 'gho_nogrouptoken',
      tokenType: 'bearer',
      scope: 'user:email',
      // No groupId
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 86400000),
      lastAccessedAt: new Date(),
    }

    it('should throw error when saving without session', async () => {
      await expect(storage.save('tasks/task-123', { title: 'Test' })).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should throw error when saving without group', async () => {
      await SessionContext.run(sessionWithoutGroup, async () => {
        await expect(storage.save('tasks/task-123', { title: 'Test' })).rejects.toThrow(
          'User 789 is not assigned to any group',
        )
      })
    })

    it('should throw error when getting without session', async () => {
      await expect(storage.get('tasks/task-123')).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should throw error when getting without group', async () => {
      await SessionContext.run(sessionWithoutGroup, async () => {
        await expect(storage.get('tasks/task-123')).rejects.toThrow(
          'User 789 is not assigned to any group',
        )
      })
    })

    it('should throw error when listing without session', async () => {
      await expect(storage.list('tasks/')).rejects.toThrow('Unauthorized: No user session found')
    })

    it('should throw error when listing without group', async () => {
      await SessionContext.run(sessionWithoutGroup, async () => {
        await expect(storage.list('tasks/')).rejects.toThrow(
          'User 789 is not assigned to any group',
        )
      })
    })

    it('should throw error when deleting without session', async () => {
      await expect(storage.delete('tasks/task-123')).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should throw error when deleting without group', async () => {
      await SessionContext.run(sessionWithoutGroup, async () => {
        await expect(storage.delete('tasks/task-123')).rejects.toThrow(
          'User 789 is not assigned to any group',
        )
      })
    })

    it('should throw error when checking existence without session', async () => {
      await expect(storage.exists('tasks/task-123')).rejects.toThrow(
        'Unauthorized: No user session found',
      )
    })

    it('should throw error when checking existence without group', async () => {
      await SessionContext.run(sessionWithoutGroup, async () => {
        await expect(storage.exists('tasks/task-123')).rejects.toThrow(
          'User 789 is not assigned to any group',
        )
      })
    })

    it('should allow session operations without session (bootstrap case)', async () => {
      // Sessions can be accessed without a session context (needed for auth middleware)
      await storage.save('sessions/session-123', { userId: 456 })
      const retrieved = await storage.get('sessions/session-123')
      expect(retrieved).toEqual({ userId: 456 })

      const keys = await storage.list('sessions/')
      expect(keys).toContain('sessions/session-123')
    })

    it('should allow user operations without group (needed for user management)', async () => {
      await SessionContext.run(testSession, async () => {
        await storage.save('users/user-456', { username: 'testuser' })
        const retrieved = await storage.get('users/user-456')
        expect(retrieved).toEqual({ username: 'testuser' })

        const keys = await storage.list('users/')
        expect(keys).toContain('users/user-456')
      })
    })
  })

  describe('Type-safe convenience methods', () => {
    it('should save and retrieve repositories with group scoping', async () => {
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

        // Verify file is in group-scoped path
        const expectedPath = path.join(testDir, 'groups/team-frontend/repositories/repo-123.json')
        const exists = await fs
          .access(expectedPath)
          .then(() => true)
          .catch(() => false)
        expect(exists).toBe(true)
      })
    })

    it('should list repositories with group scoping - REGRESSION TEST', async () => {
      await SessionContext.run(testSession, async () => {
        // Create multiple repositories
        const repo1 = {
          id: 'repo-1',
          name: 'Repository 1',
          path: '/test/path1',
          useGitWorktrees: false,
          cloneOnNewTask: false,
          codeExecution: { type: 'local' as const },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        }

        const repo2 = {
          id: 'repo-2',
          name: 'Repository 2',
          path: '/test/path2',
          useGitWorktrees: false,
          cloneOnNewTask: false,
          codeExecution: { type: 'local' as const },
          variables: {},
          createdAt: new Date(),
          updatedAt: new Date(),
        }

        await storage.saveRepository(repo1)
        await storage.saveRepository(repo2)

        // List repositories - this was failing before the fix
        const keys = await storage.list('repositories/')

        // Should return both repositories
        expect(keys).toHaveLength(2)
        expect(keys).toContain('repositories/repo-1')
        expect(keys).toContain('repositories/repo-2')

        // Verify files are in the correct group-scoped path
        const repo1Path = path.join(testDir, 'groups/team-frontend/repositories/repo-1.json')
        const repo2Path = path.join(testDir, 'groups/team-frontend/repositories/repo-2.json')

        const repo1Exists = await fs
          .access(repo1Path)
          .then(() => true)
          .catch(() => false)
        const repo2Exists = await fs
          .access(repo2Path)
          .then(() => true)
          .catch(() => false)

        expect(repo1Exists).toBe(true)
        expect(repo2Exists).toBe(true)
      })
    })
  })
})
