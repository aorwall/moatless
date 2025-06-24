export interface CacheOptions {
  ttl?: number // Time to live in milliseconds
  maxSize?: number // Maximum number of entries
  checkPeriod?: number // Cleanup interval in milliseconds
}

export interface CacheEntry<T> {
  value: T
  timestamp: number
  ttl: number
  accessed: number
}

export class Cache<T = any> {
  private store = new Map<string, CacheEntry<T>>()
  private cleanupInterval?: NodeJS.Timeout

  constructor(private options: CacheOptions = {}) {
    const { checkPeriod = 60000 } = options // Default 1 minute

    if (checkPeriod > 0) {
      this.cleanupInterval = setInterval(() => {
        this.cleanup()
      }, checkPeriod)
    }
  }

  set(key: string, value: T, ttl?: number): void {
    const entry: CacheEntry<T> = {
      value,
      timestamp: Date.now(),
      ttl: ttl || this.options.ttl || 0,
      accessed: Date.now(),
    }

    // Remove oldest entries if max size exceeded
    if (this.options.maxSize && this.store.size >= this.options.maxSize) {
      this.evictOldest()
    }

    this.store.set(key, entry)
  }

  get(key: string): T | null {
    const entry = this.store.get(key)

    if (!entry) {
      return null
    }

    // Check if expired
    if (entry.ttl > 0 && Date.now() - entry.timestamp > entry.ttl) {
      this.store.delete(key)
      return null
    }

    // Update access time
    entry.accessed = Date.now()

    return entry.value
  }

  has(key: string): boolean {
    return this.get(key) !== null
  }

  delete(key: string): boolean {
    return this.store.delete(key)
  }

  clear(): void {
    this.store.clear()
  }

  size(): number {
    return this.store.size
  }

  keys(): string[] {
    return Array.from(this.store.keys())
  }

  values(): T[] {
    return Array.from(this.store.values()).map((entry) => entry.value)
  }

  entries(): Array<[string, T]> {
    return Array.from(this.store.entries()).map(([key, entry]) => [key, entry.value])
  }

  // Get or set pattern
  async getOrSet<R = T>(key: string, factory: () => Promise<R> | R, ttl?: number): Promise<R> {
    const existing = this.get(key)
    if (existing !== null) {
      return existing as R
    }

    const value = await factory()
    this.set(key, value as T, ttl)
    return value
  }

  // Memoize a function with caching
  memoize<TArgs extends any[], TReturn>(
    fn: (...args: TArgs) => Promise<TReturn> | TReturn,
    keyGenerator: (...args: TArgs) => string = (...args) => JSON.stringify(args),
    ttl?: number,
  ): (...args: TArgs) => Promise<TReturn> {
    return async (...args: TArgs): Promise<TReturn> => {
      const key = keyGenerator(...args)
      return this.getOrSet<TReturn>(key, () => fn(...args), ttl)
    }
  }

  private cleanup(): void {
    const now = Date.now()

    for (const [key, entry] of this.store.entries()) {
      if (entry.ttl > 0 && now - entry.timestamp > entry.ttl) {
        this.store.delete(key)
      }
    }
  }

  private evictOldest(): void {
    let oldestKey = ''
    let oldestTime = Infinity

    for (const [key, entry] of this.store.entries()) {
      if (entry.accessed < oldestTime) {
        oldestTime = entry.accessed
        oldestKey = key
      }
    }

    if (oldestKey) {
      this.store.delete(oldestKey)
    }
  }

  destroy(): void {
    if (this.cleanupInterval) {
      clearInterval(this.cleanupInterval)
    }
    this.clear()
  }
}

// Global cache instances
export const memoryCache = new Cache({
  ttl: 300000, // 5 minutes
  maxSize: 1000,
  checkPeriod: 60000, // 1 minute
})

export const shortTermCache = new Cache({
  ttl: 60000, // 1 minute
  maxSize: 500,
  checkPeriod: 30000, // 30 seconds
})

export const longTermCache = new Cache({
  ttl: 3600000, // 1 hour
  maxSize: 100,
  checkPeriod: 300000, // 5 minutes
})
