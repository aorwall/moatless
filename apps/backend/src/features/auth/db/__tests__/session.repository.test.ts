import type { UserSession } from '@moatless/types'
import type { BunSQLiteDatabase } from 'drizzle-orm/bun-sqlite'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Database } from '../../../../infrastructure/database/database'
import { SessionRepository } from '../session.repository'

describe('SessionRepository', () => {
  let sessionRepository: SessionRepository
  let mockDatabase: Database
  let mockDrizzle: Partial<BunSQLiteDatabase>

  beforeEach(() => {
    // Create mock Drizzle instance
    mockDrizzle = {
      insert: vi.fn().mockReturnValue({
        values: vi.fn().mockReturnValue({
          onConflictDoUpdate: vi.fn().mockResolvedValue(undefined),
        }),
      }),
      select: vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockReturnValue({
            limit: vi.fn().mockResolvedValue([]),
          }),
        }),
      }),
      delete: vi.fn().mockReturnValue({
        where: vi.fn().mockResolvedValue({ changes: 0 }),
      }),
    }

    // Create mock Database
    mockDatabase = {
      drizzle: mockDrizzle as BunSQLiteDatabase,
      migrate: vi.fn().mockResolvedValue(undefined),
      transaction: vi.fn(),
      close: vi.fn().mockResolvedValue(undefined),
    }

    // Create repository instance
    sessionRepository = new SessionRepository(mockDatabase)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('saveSession', () => {
    it('should insert session with upsert logic', async () => {
      const session: UserSession = {
        id: 'session-123',
        provider: 'github',
        userId: 'user-123',
        userLogin: 'johndoe',
        userEmail: 'john@example.com',
        userName: 'John Doe',
        avatarUrl: 'https://example.com/avatar.jpg',
        accessToken: 'github_token_123',
        idToken: 'id_token_123',
        tokenType: 'Bearer',
        scope: 'user:email',
        groupId: 'group-456',
        createdAt: new Date('2025-01-01'),
        expiresAt: new Date('2025-01-08'),
      }

      await sessionRepository.saveSession(session)

      expect(mockDrizzle.insert).toHaveBeenCalledOnce()
    })

    it('should handle session with optional fields as null', async () => {
      const session: UserSession = {
        id: 'session-456',
        provider: 'auth0',
        userId: 'user-456',
        userLogin: 'janedoe',
        userEmail: 'jane@example.com',
        userName: 'Jane Doe',
        avatarUrl: null,
        createdAt: new Date('2025-01-01'),
        expiresAt: new Date('2025-01-08'),
      }

      await sessionRepository.saveSession(session)

      expect(mockDrizzle.insert).toHaveBeenCalledOnce()
    })
  })

  describe('getSession', () => {
    it('should return session when found', async () => {
      const mockRow = {
        id: 'session-123',
        provider: 'github',
        userId: 'user-123',
        userLogin: 'johndoe',
        userEmail: 'john@example.com',
        userName: 'John Doe',
        avatarUrl: 'https://example.com/avatar.jpg',
        accessToken: 'github_token_123',
        idToken: 'id_token_123',
        tokenType: 'Bearer',
        scope: 'user:email',
        groupId: 'group-456',
        createdAt: new Date('2025-01-01'),
        expiresAt: new Date('2025-01-08'),
      }

      // Mock the select chain to return our session
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockReturnValue({
            limit: vi.fn().mockResolvedValue([mockRow]),
          }),
        }),
      })

      const result = await sessionRepository.getSession('session-123')

      expect(result).toEqual(mockRow)
      expect(mockDrizzle.select).toHaveBeenCalledOnce()
    })

    it('should return null when session not found', async () => {
      // Mock returns empty array
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockReturnValue({
            limit: vi.fn().mockResolvedValue([]),
          }),
        }),
      })

      const result = await sessionRepository.getSession('nonexistent')

      expect(result).toBeNull()
    })
  })

  describe('deleteSession', () => {
    it('should delete session by id', async () => {
      await sessionRepository.deleteSession('session-123')

      expect(mockDrizzle.delete).toHaveBeenCalledOnce()
    })
  })

  describe('getExpiredSessions', () => {
    it('should return expired sessions', async () => {
      const mockRows = [
        {
          id: 'session-1',
          provider: 'github',
          userId: 'user-1',
          userLogin: 'user1',
          userEmail: 'user1@example.com',
          userName: 'User One',
          avatarUrl: null,
          accessToken: 'token1',
          idToken: null,
          tokenType: 'Bearer',
          scope: 'user:email',
          groupId: null,
          createdAt: new Date('2024-12-01'),
          expiresAt: new Date('2024-12-08'),
        },
        {
          id: 'session-2',
          provider: 'auth0',
          userId: 'user-2',
          userLogin: 'user2',
          userEmail: 'user2@example.com',
          userName: 'User Two',
          avatarUrl: null,
          accessToken: 'token2',
          idToken: null,
          tokenType: 'Bearer',
          scope: 'openid profile',
          groupId: null,
          createdAt: new Date('2024-12-02'),
          expiresAt: new Date('2024-12-09'),
        },
      ]

      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockResolvedValue(mockRows),
        }),
      })

      const result = await sessionRepository.getExpiredSessions()

      expect(result).toHaveLength(2)
      expect(result[0]!.id).toBe('session-1')
      expect(result[1]!.id).toBe('session-2')
    })

    it('should return empty array when no expired sessions exist', async () => {
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockResolvedValue([]),
        }),
      })

      const result = await sessionRepository.getExpiredSessions()

      expect(result).toEqual([])
    })
  })

  describe('deleteExpiredSessions', () => {
    it('should delete expired sessions and return count', async () => {
      mockDrizzle.delete = vi.fn().mockReturnValue({
        where: vi.fn().mockResolvedValue({ changes: 5 }),
      })

      const result = await sessionRepository.deleteExpiredSessions()

      expect(result).toBe(5)
      expect(mockDrizzle.delete).toHaveBeenCalledOnce()
    })

    it('should return 0 when no expired sessions to delete', async () => {
      mockDrizzle.delete = vi.fn().mockReturnValue({
        where: vi.fn().mockResolvedValue({ changes: 0 }),
      })

      const result = await sessionRepository.deleteExpiredSessions()

      expect(result).toBe(0)
    })
  })

  describe('getAllSessions', () => {
    it('should return all sessions', async () => {
      const mockRows = [
        {
          id: 'session-1',
          provider: 'github',
          userId: 'user-1',
          userLogin: 'user1',
          userEmail: 'user1@example.com',
          userName: 'User One',
          avatarUrl: null,
          accessToken: 'token1',
          idToken: null,
          tokenType: 'Bearer',
          scope: 'user:email',
          groupId: null,
          createdAt: new Date('2025-01-01'),
          expiresAt: new Date('2025-01-08'),
        },
        {
          id: 'session-2',
          provider: 'auth0',
          userId: 'user-2',
          userLogin: 'user2',
          userEmail: 'user2@example.com',
          userName: 'User Two',
          avatarUrl: null,
          accessToken: 'token2',
          idToken: null,
          tokenType: 'Bearer',
          scope: 'openid profile',
          groupId: null,
          createdAt: new Date('2025-01-02'),
          expiresAt: new Date('2025-01-09'),
        },
      ]

      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockResolvedValue(mockRows),
      })

      const result = await sessionRepository.getAllSessions()

      expect(result).toHaveLength(2)
      expect(result[0]!.id).toBe('session-1')
      expect(result[1]!.id).toBe('session-2')
    })

    it('should return empty array when no sessions exist', async () => {
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockResolvedValue([]),
      })

      const result = await sessionRepository.getAllSessions()

      expect(result).toEqual([])
    })
  })

  describe('getSessionsByUserId', () => {
    it('should return sessions for specified user', async () => {
      const mockRows = [
        {
          id: 'session-1',
          provider: 'github',
          userId: 'user-123',
          userLogin: 'johndoe',
          userEmail: 'john@example.com',
          userName: 'John Doe',
          avatarUrl: null,
          accessToken: 'token1',
          idToken: null,
          tokenType: 'Bearer',
          scope: 'user:email',
          groupId: null,
          createdAt: new Date('2025-01-01'),
          expiresAt: new Date('2025-01-08'),
        },
        {
          id: 'session-2',
          provider: 'github',
          userId: 'user-123',
          userLogin: 'johndoe',
          userEmail: 'john@example.com',
          userName: 'John Doe',
          avatarUrl: null,
          accessToken: 'token2',
          idToken: null,
          tokenType: 'Bearer',
          scope: 'user:email',
          groupId: null,
          createdAt: new Date('2025-01-02'),
          expiresAt: new Date('2025-01-09'),
        },
      ]

      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockResolvedValue(mockRows),
        }),
      })

      const result = await sessionRepository.getSessionsByUserId('user-123')

      expect(result).toHaveLength(2)
      expect(result[0]!.userId).toBe('user-123')
      expect(result[1]!.userId).toBe('user-123')
    })

    it('should return empty array when user has no sessions', async () => {
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockResolvedValue([]),
        }),
      })

      const result = await sessionRepository.getSessionsByUserId('user-nonexistent')

      expect(result).toEqual([])
    })
  })
})
