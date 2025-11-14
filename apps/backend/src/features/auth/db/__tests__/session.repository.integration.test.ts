/**
 * Integration tests for SessionRepository with real SQLite database.
 *
 * Uses Bun's native SQLite implementation (bun:sqlite) via Drizzle ORM.
 * These tests create a temporary database for each test suite to ensure isolation.
 */
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { UserSession } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { Database } from '../../../../infrastructure/database/database'
import { createDrizzleDatabase } from '../../../../infrastructure/database/drizzle'
import { SessionRepository } from '../session.repository'

describe('SessionRepository Integration Tests', () => {
  let sessionRepository: SessionRepository
  let database: Database
  let tempDir: string
  let dbPath: string

  beforeEach(async () => {
    // Create temporary directory for test database
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-session-test-'))
    dbPath = path.join(tempDir, 'test.db')

    // Create real database instance
    database = createDrizzleDatabase(dbPath)

    // Run migrations to create schema
    await database.migrate()

    // Create repository instance
    sessionRepository = new SessionRepository(database)
  })

  afterEach(async () => {
    // Close database connection
    await database.close()

    // Clean up temporary directory
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  describe('saveSession and getSession', () => {
    it('should save and retrieve a session with all fields', async () => {
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
        createdAt: new Date('2025-01-01T00:00:00Z'),
        expiresAt: new Date('2025-01-08T00:00:00Z'),
      }

      // Save session
      await sessionRepository.saveSession(session)

      // Retrieve session
      const retrieved = await sessionRepository.getSession('session-123')

      expect(retrieved).not.toBeNull()
      expect(retrieved?.id).toBe('session-123')
      expect(retrieved?.provider).toBe('github')
      expect(retrieved?.userId).toBe('user-123')
      expect(retrieved?.userLogin).toBe('johndoe')
      expect(retrieved?.userEmail).toBe('john@example.com')
      expect(retrieved?.userName).toBe('John Doe')
      expect(retrieved?.avatarUrl).toBe('https://example.com/avatar.jpg')
      expect(retrieved?.accessToken).toBe('github_token_123')
      expect(retrieved?.idToken).toBe('id_token_123')
      expect(retrieved?.tokenType).toBe('Bearer')
      expect(retrieved?.scope).toBe('user:email')
      expect(retrieved?.groupId).toBe('group-456')
      expect(retrieved?.createdAt).toEqual(session.createdAt)
      expect(retrieved?.expiresAt).toEqual(session.expiresAt)
    })

    it('should handle upsert - update existing session', async () => {
      const session: UserSession = {
        id: 'session-456',
        provider: 'github',
        userId: 'user-456',
        userLogin: 'janedoe',
        userEmail: 'jane@example.com',
        userName: 'Jane Doe',
        avatarUrl: null,
        createdAt: new Date('2025-01-01'),
        expiresAt: new Date('2025-01-08'),
      }

      // Save session first time
      await sessionRepository.saveSession(session)

      // Update session
      const updatedSession: UserSession = {
        ...session,
        accessToken: 'new_token',
        expiresAt: new Date('2025-01-15'),
      }

      await sessionRepository.saveSession(updatedSession)

      // Retrieve and verify update
      const retrieved = await sessionRepository.getSession('session-456')

      expect(retrieved).not.toBeNull()
      expect(retrieved?.accessToken).toBe('new_token')
      expect(retrieved?.expiresAt).toEqual(new Date('2025-01-15'))
    })

    it('should handle sessions with null optional fields', async () => {
      const session: UserSession = {
        id: 'session-789',
        provider: 'auth0',
        userId: 'user-789',
        userLogin: 'testuser',
        userEmail: 'test@example.com',
        userName: 'Test User',
        avatarUrl: null,
        createdAt: new Date('2025-01-01'),
        expiresAt: new Date('2025-01-08'),
      }

      await sessionRepository.saveSession(session)

      const retrieved = await sessionRepository.getSession('session-789')

      expect(retrieved).not.toBeNull()
      // SQLite NULL fields are returned as null for string | null types
      expect(retrieved?.avatarUrl).toBeNull()
      // Optional fields (string | undefined) are converted to undefined
      expect(retrieved?.accessToken).toBeUndefined()
      expect(retrieved?.idToken).toBeUndefined()
      expect(retrieved?.tokenType).toBeUndefined()
      expect(retrieved?.scope).toBeUndefined()
      expect(retrieved?.groupId).toBeUndefined()
    })

    it('should return null when session not found', async () => {
      const retrieved = await sessionRepository.getSession('nonexistent')

      expect(retrieved).toBeNull()
    })
  })

  describe('deleteSession', () => {
    it('should delete a session', async () => {
      const session: UserSession = {
        id: 'session-delete-1',
        provider: 'github',
        userId: 'user-1',
        userLogin: 'user1',
        userEmail: 'user1@example.com',
        userName: 'User One',
        avatarUrl: null,
        createdAt: new Date('2025-01-01'),
        expiresAt: new Date('2025-01-08'),
      }

      // Save session
      await sessionRepository.saveSession(session)

      // Verify it exists
      let retrieved = await sessionRepository.getSession('session-delete-1')
      expect(retrieved).not.toBeNull()

      // Delete session
      await sessionRepository.deleteSession('session-delete-1')

      // Verify it's gone
      retrieved = await sessionRepository.getSession('session-delete-1')
      expect(retrieved).toBeNull()
    })
  })

  describe('getExpiredSessions and deleteExpiredSessions', () => {
    it('should identify expired sessions', async () => {
      const now = new Date()
      const pastDate = new Date(now.getTime() - 24 * 60 * 60 * 1000) // 1 day ago
      const futureDate = new Date(now.getTime() + 24 * 60 * 60 * 1000) // 1 day from now

      // Create expired session
      const expiredSession: UserSession = {
        id: 'session-expired-1',
        provider: 'github',
        userId: 'user-1',
        userLogin: 'user1',
        userEmail: 'user1@example.com',
        userName: 'User One',
        avatarUrl: null,
        createdAt: new Date('2024-12-01'),
        expiresAt: pastDate,
      }

      // Create active session
      const activeSession: UserSession = {
        id: 'session-active-1',
        provider: 'github',
        userId: 'user-2',
        userLogin: 'user2',
        userEmail: 'user2@example.com',
        userName: 'User Two',
        avatarUrl: null,
        createdAt: now,
        expiresAt: futureDate,
      }

      await sessionRepository.saveSession(expiredSession)
      await sessionRepository.saveSession(activeSession)

      // Get expired sessions
      const expired = await sessionRepository.getExpiredSessions()

      expect(expired).toHaveLength(1)
      expect(expired[0]?.id).toBe('session-expired-1')
    })

    it('should delete expired sessions and return count', async () => {
      const now = new Date()
      const pastDate = new Date(now.getTime() - 24 * 60 * 60 * 1000) // 1 day ago

      // Create 3 expired sessions
      for (let i = 1; i <= 3; i++) {
        const session: UserSession = {
          id: `session-expired-${i}`,
          provider: 'github',
          userId: `user-${i}`,
          userLogin: `user${i}`,
          userEmail: `user${i}@example.com`,
          userName: `User ${i}`,
          avatarUrl: null,
          createdAt: new Date('2024-12-01'),
          expiresAt: pastDate,
        }
        await sessionRepository.saveSession(session)
      }

      // Delete expired sessions
      const deletedCount = await sessionRepository.deleteExpiredSessions()

      expect(deletedCount).toBe(3)

      // Verify they're gone
      const expired = await sessionRepository.getExpiredSessions()
      expect(expired).toHaveLength(0)
    })

    it('should return 0 when no expired sessions exist', async () => {
      const deletedCount = await sessionRepository.deleteExpiredSessions()

      expect(deletedCount).toBe(0)
    })
  })

  describe('getAllSessions', () => {
    it('should return all sessions', async () => {
      // Create multiple sessions
      for (let i = 1; i <= 3; i++) {
        const session: UserSession = {
          id: `session-all-${i}`,
          provider: 'github',
          userId: `user-${i}`,
          userLogin: `user${i}`,
          userEmail: `user${i}@example.com`,
          userName: `User ${i}`,
          avatarUrl: null,
          createdAt: new Date('2025-01-01'),
          expiresAt: new Date('2025-01-08'),
        }
        await sessionRepository.saveSession(session)
      }

      const allSessions = await sessionRepository.getAllSessions()

      expect(allSessions).toHaveLength(3)
      expect(allSessions.map((s) => s.id)).toEqual([
        'session-all-1',
        'session-all-2',
        'session-all-3',
      ])
    })

    it('should return empty array when no sessions exist', async () => {
      const allSessions = await sessionRepository.getAllSessions()

      expect(allSessions).toEqual([])
    })
  })

  describe('getSessionsByUserId', () => {
    it('should return all sessions for a specific user', async () => {
      // Create sessions for user-123
      for (let i = 1; i <= 2; i++) {
        const session: UserSession = {
          id: `session-user123-${i}`,
          provider: 'github',
          userId: 'user-123',
          userLogin: 'johndoe',
          userEmail: 'john@example.com',
          userName: 'John Doe',
          avatarUrl: null,
          createdAt: new Date('2025-01-01'),
          expiresAt: new Date('2025-01-08'),
        }
        await sessionRepository.saveSession(session)
      }

      // Create session for different user
      const otherSession: UserSession = {
        id: 'session-user456-1',
        provider: 'github',
        userId: 'user-456',
        userLogin: 'janedoe',
        userEmail: 'jane@example.com',
        userName: 'Jane Doe',
        avatarUrl: null,
        createdAt: new Date('2025-01-01'),
        expiresAt: new Date('2025-01-08'),
      }
      await sessionRepository.saveSession(otherSession)

      // Get sessions for user-123
      const userSessions = await sessionRepository.getSessionsByUserId('user-123')

      expect(userSessions).toHaveLength(2)
      expect(userSessions.every((s) => s.userId === 'user-123')).toBe(true)
    })

    it('should return empty array when user has no sessions', async () => {
      const userSessions = await sessionRepository.getSessionsByUserId('nonexistent-user')

      expect(userSessions).toEqual([])
    })
  })
})
