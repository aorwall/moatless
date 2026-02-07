import { randomUUID } from 'node:crypto'
import { inject, injectable, optional } from 'inversify'
import type Redis from 'ioredis'
import { logger } from '../utils/logger'

/**
 * Symbol for injecting the Redis URL configuration
 */
export const RedisUrlSymbol = Symbol.for('RedisUrl')

/**
 * Symbol for dependency injection
 */
export const RedisLockServiceSymbol = Symbol.for('RedisLockService')

/**
 * Options for RedisLockService
 */
export interface RedisLockServiceOptions {
  /**
   * Redis connection URL (e.g., redis://localhost:6379)
   */
  redisUrl: string
}

/**
 * Result of attempting to acquire a lock
 */
export interface LockResult {
  acquired: boolean
  token: string | null
}

/**
 * RedisLockService - Distributed locking using Redis
 *
 * Implements distributed locks using Redis SET NX EX pattern.
 * Used to coordinate across multiple backend replicas to prevent
 * race conditions like duplicate pod creation.
 *
 * Lock Token Pattern:
 * - Each lock acquisition generates a unique token
 * - Release only succeeds if the token matches (prevents releasing others' locks)
 * - Tokens are UUIDs to ensure uniqueness across replicas
 *
 * TTL Strategy:
 * - Locks have a TTL to prevent deadlocks if a replica crashes
 * - Use extendLock() for long-running operations
 * - TTL should be longer than expected operation time
 */
@injectable()
export class RedisLockService {
  private redis: Redis | null = null
  private readonly redisUrl: string

  constructor(
    @inject(RedisUrlSymbol)
    @optional()
    redisUrl?: string,
  ) {
    this.redisUrl = redisUrl ?? process.env['REDIS_URL'] ?? 'redis://localhost:6379'
  }

  /**
   * Initialize the Redis connection
   */
  async initialize(): Promise<void> {
    if (this.redis) {
      logger.warn({}, 'RedisLockService: Already initialized')
      return
    }

    try {
      const { default: Redis } = await import('ioredis')

      this.redis = new Redis(this.redisUrl, {
        maxRetriesPerRequest: null,
        enableReadyCheck: true,
        lazyConnect: false,
      })

      // Wait for connection
      await new Promise<void>((resolve, reject) => {
        if (!this.redis) {
          return reject(new Error('Redis client not initialized'))
        }

        this.redis.once('ready', () => {
          logger.info({ redisUrl: this.redisUrl }, 'RedisLockService: Connected')
          resolve()
        })

        this.redis.once('error', (error) => {
          logger.error({ err: error }, 'RedisLockService: Connection error')
          reject(error)
        })
      })
    } catch (error) {
      logger.error({ err: error }, 'RedisLockService: Failed to initialize')
      throw error
    }
  }

  /**
   * Acquire a distributed lock
   *
   * @param key - Lock key (e.g., "workspace:provision:task-123")
   * @param ttlMs - Lock TTL in milliseconds (default: 120000ms = 2 minutes)
   * @returns Lock token if acquired, null if lock is held by another process
   */
  async acquireLock(key: string, ttlMs = 120_000): Promise<string | null> {
    this.ensureInitialized()

    const token = randomUUID()
    const lockKey = this.formatLockKey(key)
    const ttlSeconds = Math.ceil(ttlMs / 1000)

    try {
      // SET key value NX EX seconds
      // NX = only set if not exists
      // EX = expire in seconds
      const result = await this.redis!.set(lockKey, token, 'EX', ttlSeconds, 'NX')

      if (result === 'OK') {
        logger.debug({ key: lockKey, token, ttlMs }, 'RedisLockService: Lock acquired')
        return token
      }

      logger.debug({ key: lockKey }, 'RedisLockService: Lock already held')
      return null
    } catch (error) {
      logger.error({ err: error, key: lockKey }, 'RedisLockService: Failed to acquire lock')
      throw error
    }
  }

  /**
   * Release a distributed lock
   *
   * Only releases if the token matches (prevents releasing others' locks)
   *
   * @param key - Lock key
   * @param token - Token returned from acquireLock
   * @returns true if lock was released, false if token didn't match or lock expired
   */
  async releaseLock(key: string, token: string): Promise<boolean> {
    this.ensureInitialized()

    const lockKey = this.formatLockKey(key)

    try {
      // Use Lua script for atomic check-and-delete
      // This prevents race conditions where another process acquires the lock
      // between our GET and DEL
      const script = `
        if redis.call("get", KEYS[1]) == ARGV[1] then
          return redis.call("del", KEYS[1])
        else
          return 0
        end
      `

      const result = await this.redis!.eval(script, 1, lockKey, token)

      if (result === 1) {
        logger.debug({ key: lockKey, token }, 'RedisLockService: Lock released')
        return true
      }

      logger.debug(
        { key: lockKey, token },
        'RedisLockService: Lock release failed (token mismatch or expired)',
      )
      return false
    } catch (error) {
      logger.error({ err: error, key: lockKey }, 'RedisLockService: Failed to release lock')
      throw error
    }
  }

  /**
   * Extend lock TTL
   *
   * Only extends if the token matches (prevents extending others' locks)
   *
   * @param key - Lock key
   * @param token - Token returned from acquireLock
   * @param ttlMs - New TTL in milliseconds
   * @returns true if lock TTL was extended, false if token didn't match or lock expired
   */
  async extendLock(key: string, token: string, ttlMs: number): Promise<boolean> {
    this.ensureInitialized()

    const lockKey = this.formatLockKey(key)
    const ttlSeconds = Math.ceil(ttlMs / 1000)

    try {
      // Use Lua script for atomic check-and-extend
      const script = `
        if redis.call("get", KEYS[1]) == ARGV[1] then
          return redis.call("expire", KEYS[1], ARGV[2])
        else
          return 0
        end
      `

      const result = await this.redis!.eval(script, 1, lockKey, token, ttlSeconds)

      if (result === 1) {
        logger.debug({ key: lockKey, token, ttlMs }, 'RedisLockService: Lock extended')
        return true
      }

      logger.debug(
        { key: lockKey, token },
        'RedisLockService: Lock extend failed (token mismatch or expired)',
      )
      return false
    } catch (error) {
      logger.error({ err: error, key: lockKey }, 'RedisLockService: Failed to extend lock')
      throw error
    }
  }

  /**
   * Check if a lock is currently held
   *
   * @param key - Lock key
   * @returns true if lock exists, false otherwise
   */
  async isLocked(key: string): Promise<boolean> {
    this.ensureInitialized()

    const lockKey = this.formatLockKey(key)

    try {
      const result = await this.redis!.exists(lockKey)
      return result === 1
    } catch (error) {
      logger.error({ err: error, key: lockKey }, 'RedisLockService: Failed to check lock')
      throw error
    }
  }

  /**
   * Wait for a lock to be released
   *
   * @param key - Lock key
   * @param timeoutMs - Maximum time to wait
   * @param pollIntervalMs - Polling interval (default: 500ms)
   * @returns true if lock was released within timeout, false if timed out
   */
  async waitForUnlock(key: string, timeoutMs: number, pollIntervalMs = 500): Promise<boolean> {
    const startTime = Date.now()

    while (Date.now() - startTime < timeoutMs) {
      const isLocked = await this.isLocked(key)
      if (!isLocked) {
        return true
      }
      await this.sleep(pollIntervalMs)
    }

    logger.debug({ key, timeoutMs }, 'RedisLockService: Timed out waiting for lock release')
    return false
  }

  /**
   * Check if service is healthy
   */
  isHealthy(): boolean {
    return this.redis?.status === 'ready'
  }

  /**
   * Close the Redis connection
   */
  async close(): Promise<void> {
    if (this.redis) {
      await this.redis.quit()
      this.redis = null
      logger.info({}, 'RedisLockService: Connection closed')
    }
  }

  private formatLockKey(key: string): string {
    return `lock:${key}`
  }

  private ensureInitialized(): void {
    if (!this.redis) {
      throw new Error('RedisLockService: Not initialized. Call initialize() first.')
    }
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
