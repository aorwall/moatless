import type { User } from '@moatless/types'
import type { BunSQLiteDatabase } from 'drizzle-orm/bun-sqlite'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Database } from '../../../../infrastructure/database/database'
import { UserRepository } from '../user.repository'

describe('UserRepository', () => {
  let userRepository: UserRepository
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
        where: vi.fn().mockResolvedValue(undefined),
      }),
    }

    // Create mock Database
    mockDatabase = {
      drizzle: mockDrizzle as BunSQLiteDatabase,
      provider: 'sqlite',
      migrate: vi.fn().mockResolvedValue(undefined),
      transaction: vi.fn(),
      close: vi.fn().mockResolvedValue(undefined),
    }

    // Create repository instance
    userRepository = new UserRepository(mockDatabase)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('saveUser', () => {
    it('should insert user with upsert logic', async () => {
      const user: User = {
        id: 'user-123',
        login: 'johndoe',
        email: 'john@example.com',
        name: 'John Doe',
        avatarUrl: 'https://example.com/avatar.jpg',
        provider: 'github',
        createdAt: new Date('2025-01-01'),
        lastLoginAt: new Date('2025-01-10'),
      }

      await userRepository.saveUser(user)

      expect(mockDrizzle.insert).toHaveBeenCalledOnce()
    })

    it('should handle user with null optional fields', async () => {
      const user: User = {
        id: 'user-456',
        login: 'janedoe',
        email: null,
        name: null,
        avatarUrl: null,
        provider: 'auth0',
        createdAt: new Date('2025-01-01'),
        lastLoginAt: new Date('2025-01-10'),
      }

      await userRepository.saveUser(user)

      expect(mockDrizzle.insert).toHaveBeenCalledOnce()
    })
  })

  describe('getUser', () => {
    it('should return user when found', async () => {
      const mockRow = {
        id: 'user-123',
        login: 'johndoe',
        email: 'john@example.com',
        name: 'John Doe',
        avatarUrl: 'https://example.com/avatar.jpg',
        provider: 'github',
        createdAt: new Date('2025-01-01'),
        lastLoginAt: new Date('2025-01-10'),
      }

      // Mock the select chain to return our user
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockReturnValue({
            limit: vi.fn().mockResolvedValue([mockRow]),
          }),
        }),
      })

      const result = await userRepository.getUser('user-123')

      expect(result).toEqual(mockRow)
      expect(mockDrizzle.select).toHaveBeenCalledOnce()
    })

    it('should return null when user not found', async () => {
      // Mock returns empty array
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockReturnValue({
            limit: vi.fn().mockResolvedValue([]),
          }),
        }),
      })

      const result = await userRepository.getUser('nonexistent')

      expect(result).toBeNull()
    })
  })

  describe('deleteUser', () => {
    it('should delete user by id', async () => {
      await userRepository.deleteUser('user-123')

      expect(mockDrizzle.delete).toHaveBeenCalledOnce()
    })
  })

  describe('getAllUsers', () => {
    it('should return all users', async () => {
      const mockRows = [
        {
          id: 'user-1',
          login: 'user1',
          email: 'user1@example.com',
          name: 'User One',
          avatarUrl: null,
          provider: 'github',
          createdAt: new Date('2025-01-01'),
          lastLoginAt: new Date('2025-01-10'),
        },
        {
          id: 'user-2',
          login: 'user2',
          email: 'user2@example.com',
          name: 'User Two',
          avatarUrl: null,
          provider: 'auth0',
          createdAt: new Date('2025-01-02'),
          lastLoginAt: new Date('2025-01-11'),
        },
      ]

      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockResolvedValue(mockRows),
      })

      const result = await userRepository.getAllUsers()

      expect(result).toHaveLength(2)
      expect(result[0]!.id).toBe('user-1')
      expect(result[1]!.id).toBe('user-2')
    })

    it('should return empty array when no users exist', async () => {
      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockResolvedValue([]),
      })

      const result = await userRepository.getAllUsers()

      expect(result).toEqual([])
    })
  })

  describe('getUsersByProvider', () => {
    it('should return users for specified provider', async () => {
      const mockRows = [
        {
          id: 'user-1',
          login: 'user1',
          email: 'user1@example.com',
          name: 'User One',
          avatarUrl: null,
          provider: 'github',
          createdAt: new Date('2025-01-01'),
          lastLoginAt: new Date('2025-01-10'),
        },
      ]

      mockDrizzle.select = vi.fn().mockReturnValue({
        from: vi.fn().mockReturnValue({
          where: vi.fn().mockResolvedValue(mockRows),
        }),
      })

      const result = await userRepository.getUsersByProvider('github')

      expect(result).toHaveLength(1)
      expect(result[0]!.provider).toBe('github')
    })
  })
})
