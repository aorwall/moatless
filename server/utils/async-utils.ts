export interface RetryOptions {
  maxAttempts: number
  baseDelay: number
  maxDelay: number
  exponentialBackoff: boolean
  jitter: boolean
  retryIf?: (error: Error) => boolean
  onRetry?: (attempt: number, error: Error) => void
}

export class TimeoutError extends Error {
  constructor(
    message: string,
    public timeout: number,
  ) {
    super(message)
    this.name = 'TimeoutError'
  }
}

export class RetryError extends Error {
  constructor(
    message: string,
    public attempts: number,
    public lastError: Error,
  ) {
    super(message)
    this.name = 'RetryError'
  }
}

/**
 * Retry a function with exponential backoff
 */
export async function retry<T>(
  fn: () => Promise<T>,
  options: Partial<RetryOptions> = {},
): Promise<T> {
  const {
    maxAttempts = 3,
    baseDelay = 1000,
    maxDelay = 10000,
    exponentialBackoff = true,
    jitter = true,
    retryIf = () => true,
    onRetry,
  } = options

  let lastError: Error = new Error('Unknown error')

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await fn()
    } catch (error) {
      lastError = error as Error

      if (attempt === maxAttempts || !retryIf(lastError)) {
        throw new RetryError(
          `Failed after ${attempt} attempts: ${lastError.message}`,
          attempt,
          lastError,
        )
      }

      if (onRetry) {
        onRetry(attempt, lastError)
      }

      // Calculate delay
      let delay = baseDelay
      if (exponentialBackoff) {
        delay = Math.min(baseDelay * 2 ** (attempt - 1), maxDelay)
      }

      // Add jitter
      if (jitter) {
        delay += Math.random() * delay * 0.1
      }

      await sleep(delay)
    }
  }

  throw lastError
}

/**
 * Add timeout to a promise
 */
export async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  timeoutMessage?: string,
): Promise<T> {
  const timeoutPromise = new Promise<never>((_, reject) => {
    setTimeout(() => {
      reject(
        new TimeoutError(timeoutMessage || `Operation timed out after ${timeoutMs}ms`, timeoutMs),
      )
    }, timeoutMs)
  })

  return Promise.race([promise, timeoutPromise])
}

/**
 * Sleep for specified milliseconds
 */
export async function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/**
 * Debounce function calls
 */
export function debounce<T extends (...args: any[]) => any>(
  func: T,
  wait: number,
  immediate: boolean = false,
): T {
  let timeout: NodeJS.Timeout | null = null

  return ((...args: Parameters<T>) => {
    const later = () => {
      timeout = null
      if (!immediate) func(...args)
    }

    const callNow = immediate && !timeout

    if (timeout) clearTimeout(timeout)
    timeout = setTimeout(later, wait)

    if (callNow) func(...args)
  }) as T
}

/**
 * Throttle function calls
 */
export function throttle<T extends (...args: any[]) => any>(func: T, limit: number): T {
  let inThrottle: boolean = false

  return ((...args: Parameters<T>) => {
    if (!inThrottle) {
      func(...args)
      inThrottle = true
      setTimeout(() => (inThrottle = false), limit)
    }
  }) as T
}

/**
 * Execute promises in batches with concurrency limit
 */
export async function batchExecute<T, R>(
  items: T[],
  executor: (item: T) => Promise<R>,
  batchSize: number = 10,
): Promise<R[]> {
  const results: R[] = []

  for (let i = 0; i < items.length; i += batchSize) {
    const batch = items.slice(i, i + batchSize)
    const batchPromises = batch.map(executor)
    const batchResults = await Promise.all(batchPromises)
    results.push(...batchResults)
  }

  return results
}

/**
 * Execute promises with concurrency limit
 */
export async function concurrent<T, R>(
  items: T[],
  executor: (item: T) => Promise<R>,
  concurrency: number = 10,
): Promise<R[]> {
  const results: R[] = new Array(items.length)
  const executing: Promise<void>[] = []

  for (let i = 0; i < items.length; i++) {
    const promise = executor(items[i]).then((result) => {
      results[i] = result
    })

    executing.push(promise)

    if (executing.length >= concurrency) {
      await Promise.race(executing)
      const completedIndex = executing.findIndex((p) =>
        p.then(
          () => true,
          () => true,
        ),
      )
      if (completedIndex !== -1) {
        executing.splice(completedIndex, 1)
      }
    }
  }

  await Promise.all(executing)
  return results
}

/**
 * Create a cancelable promise
 */
export function makeCancelable<T>(promise: Promise<T>): {
  promise: Promise<T>
  cancel: () => void
  isCanceled: () => boolean
} {
  let isCanceled = false

  const wrappedPromise = new Promise<T>((resolve, reject) => {
    promise
      .then((value) => {
        if (!isCanceled) resolve(value)
      })
      .catch((error) => {
        if (!isCanceled) reject(error)
      })
  })

  return {
    promise: wrappedPromise,
    cancel: () => {
      isCanceled = true
    },
    isCanceled: () => isCanceled,
  }
}

/**
 * Memoize async function results
 */
export function memoizeAsync<T extends (...args: any[]) => Promise<any>>(
  fn: T,
  keyGenerator: (...args: Parameters<T>) => string = (...args) => JSON.stringify(args),
  ttl: number = 0,
): T {
  const cache = new Map<string, { value: any; timestamp: number }>()

  return ((...args: Parameters<T>) => {
    const key = keyGenerator(...args)
    const cached = cache.get(key)

    if (cached && (ttl === 0 || Date.now() - cached.timestamp < ttl)) {
      return cached.value
    }

    const promise = fn(...args)
    cache.set(key, { value: promise, timestamp: Date.now() })

    // Clean up cache on error
    promise.catch(() => cache.delete(key))

    return promise
  }) as unknown as T
}
