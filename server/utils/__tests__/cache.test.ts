import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Cache, longTermCache, memoryCache, shortTermCache } from '../cache.js'

describe('Cache', () => {
  let cache: Cache<any>

  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
    if (cache) {
      cache.destroy()
    }
  })

  describe('basic operations', () => {
    it('should set and get values', () => {
      cache = new Cache()

      cache.set('key1', 'value1')
      cache.set('key2', { foo: 'bar' })

      expect(cache.get('key1')).toBe('value1')
      expect(cache.get('key2')).toEqual({ foo: 'bar' })
    })

    it('should return null for non-existent keys', () => {
      cache = new Cache()

      expect(cache.get('non-existent')).toBe(null)
    })

    it('should check if key exists', () => {
      cache = new Cache()

      cache.set('exists', 'value')

      expect(cache.has('exists')).toBe(true)
      expect(cache.has('not-exists')).toBe(false)
    })

    it('should delete keys', () => {
      cache = new Cache()

      cache.set('key', 'value')
      expect(cache.has('key')).toBe(true)

      const deleted = cache.delete('key')
      expect(deleted).toBe(true)
      expect(cache.has('key')).toBe(false)
    })

    it('should clear all entries', () => {
      cache = new Cache()

      cache.set('key1', 'value1')
      cache.set('key2', 'value2')

      cache.clear()

      expect(cache.size()).toBe(0)
      expect(cache.has('key1')).toBe(false)
      expect(cache.has('key2')).toBe(false)
    })

    it('should get cache size', () => {
      cache = new Cache()

      expect(cache.size()).toBe(0)

      cache.set('key1', 'value1')
      cache.set('key2', 'value2')

      expect(cache.size()).toBe(2)
    })

    it('should get all keys', () => {
      cache = new Cache()

      cache.set('key1', 'value1')
      cache.set('key2', 'value2')

      const keys = cache.keys()
      expect(keys).toContain('key1')
      expect(keys).toContain('key2')
      expect(keys).toHaveLength(2)
    })

    it('should get all values', () => {
      cache = new Cache()

      cache.set('key1', 'value1')
      cache.set('key2', 'value2')

      const values = cache.values()
      expect(values).toContain('value1')
      expect(values).toContain('value2')
      expect(values).toHaveLength(2)
    })

    it('should get all entries', () => {
      cache = new Cache()

      cache.set('key1', 'value1')
      cache.set('key2', 'value2')

      const entries = cache.entries()
      expect(entries).toContainEqual(['key1', 'value1'])
      expect(entries).toContainEqual(['key2', 'value2'])
      expect(entries).toHaveLength(2)
    })
  })

  describe('TTL functionality', () => {
    it('should expire entries after TTL', () => {
      cache = new Cache({ ttl: 1000 })

      cache.set('key', 'value')
      expect(cache.get('key')).toBe('value')

      vi.advanceTimersByTime(999)
      expect(cache.get('key')).toBe('value')

      vi.advanceTimersByTime(2)
      expect(cache.get('key')).toBe(null)
    })

    it('should use custom TTL per entry', () => {
      cache = new Cache({ ttl: 5000 })

      cache.set('key1', 'value1', 1000)
      cache.set('key2', 'value2') // Uses default TTL

      vi.advanceTimersByTime(1001)

      expect(cache.get('key1')).toBe(null)
      expect(cache.get('key2')).toBe('value2')
    })

    it('should not expire entries with zero TTL', () => {
      cache = new Cache({ ttl: 0 })

      cache.set('key', 'value')

      vi.advanceTimersByTime(100000)

      expect(cache.get('key')).toBe('value')
    })
  })

  describe('max size functionality', () => {
    it('should evict oldest entries when max size exceeded', () => {
      cache = new Cache({ maxSize: 2 })

      cache.set('key1', 'value1')
      cache.set('key2', 'value2')

      // Access key1 to make it more recent
      cache.get('key1')

      // This should evict key2 (least recently accessed)
      cache.set('key3', 'value3')

      expect(cache.has('key1')).toBe(true)
      expect(cache.has('key2')).toBe(false)
      expect(cache.has('key3')).toBe(true)
    })
  })

  describe('cleanup functionality', () => {
    it('should automatically clean up expired entries', () => {
      cache = new Cache({
        ttl: 1000,
        checkPeriod: 500,
      })

      cache.set('key1', 'value1')
      cache.set('key2', 'value2')

      vi.advanceTimersByTime(1500)

      // After cleanup, expired entries should be gone
      expect(cache.size()).toBe(0)
    })
  })

  describe('getOrSet pattern', () => {
    it('should get existing value without calling factory', async () => {
      cache = new Cache()

      cache.set('key', 'existing')

      const factory = vi.fn().mockResolvedValue('new value')
      const value = await cache.getOrSet('key', factory)

      expect(value).toBe('existing')
      expect(factory).not.toHaveBeenCalled()
    })

    it('should call factory and cache result for new key', async () => {
      cache = new Cache()

      const factory = vi.fn().mockResolvedValue('new value')
      const value = await cache.getOrSet('key', factory)

      expect(value).toBe('new value')
      expect(factory).toHaveBeenCalledTimes(1)
      expect(cache.get('key')).toBe('new value')
    })

    it('should handle sync factory functions', async () => {
      cache = new Cache()

      const factory = () => 'sync value'
      const value = await cache.getOrSet('key', factory)

      expect(value).toBe('sync value')
      expect(cache.get('key')).toBe('sync value')
    })
  })

  describe('memoize functionality', () => {
    it('should memoize function results', async () => {
      cache = new Cache()

      const fn = vi.fn().mockImplementation((x: number) => x * 2)
      const memoized = cache.memoize(fn)

      const result1 = await memoized(5)
      const result2 = await memoized(5)
      const result3 = await memoized(10)

      expect(result1).toBe(10)
      expect(result2).toBe(10)
      expect(result3).toBe(20)
      expect(fn).toHaveBeenCalledTimes(2) // Only for unique arguments
    })

    it('should use custom key generator', async () => {
      cache = new Cache()

      const fn = vi.fn().mockImplementation((obj: any) => obj.value)
      const keyGen = (obj: any) => obj.id
      const memoized = cache.memoize(fn, keyGen)

      await memoized({ id: '1', value: 10 })
      await memoized({ id: '1', value: 20 }) // Same ID, should use cache

      expect(fn).toHaveBeenCalledTimes(1)
    })

    it('should respect TTL in memoization', async () => {
      cache = new Cache()

      const fn = vi.fn().mockImplementation((x: number) => x * 2)
      const memoized = cache.memoize(fn, undefined, 1000)

      await memoized(5)
      expect(fn).toHaveBeenCalledTimes(1)

      vi.advanceTimersByTime(500)
      await memoized(5)
      expect(fn).toHaveBeenCalledTimes(1) // Still cached

      vi.advanceTimersByTime(501)
      await memoized(5)
      expect(fn).toHaveBeenCalledTimes(2) // Cache expired
    })
  })

  describe('global cache instances', () => {
    it('should have different configurations', () => {
      expect(memoryCache).toBeDefined()
      expect(shortTermCache).toBeDefined()
      expect(longTermCache).toBeDefined()

      // Test that they work independently
      memoryCache.set('key', 'memory')
      shortTermCache.set('key', 'short')
      longTermCache.set('key', 'long')

      expect(memoryCache.get('key')).toBe('memory')
      expect(shortTermCache.get('key')).toBe('short')
      expect(longTermCache.get('key')).toBe('long')
    })
  })
})
