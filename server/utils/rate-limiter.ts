export interface RateLimitOptions {
  windowMs: number;
  maxRequests: number;
  keyGenerator?: (req: any) => string;
  skipSuccessfulRequests?: boolean;
  skipFailedRequests?: boolean;
  onLimitReached?: (req: any) => void;
}

export interface RateLimitInfo {
  limit: number;
  current: number;
  remaining: number;
  resetTime: Date;
}

class RateLimitStore {
  private store = new Map<string, { count: number; resetTime: number }>();
  
  async get(key: string): Promise<{ count: number; resetTime: number } | null> {
    return this.store.get(key) || null;
  }
  
  async set(key: string, value: { count: number; resetTime: number }): Promise<void> {
    this.store.set(key, value);
  }
  
  async increment(key: string, windowMs: number): Promise<{ count: number; resetTime: number }> {
    const now = Date.now();
    const existing = this.store.get(key);
    
    if (!existing || now > existing.resetTime) {
      // Create new window
      const value = { count: 1, resetTime: now + windowMs };
      this.store.set(key, value);
      return value;
    } else {
      // Increment existing window
      const value = { count: existing.count + 1, resetTime: existing.resetTime };
      this.store.set(key, value);
      return value;
    }
  }
  
  async cleanup(): Promise<void> {
    const now = Date.now();
    for (const [key, value] of this.store.entries()) {
      if (now > value.resetTime) {
        this.store.delete(key);
      }
    }
  }
}

export class RateLimiter {
  private store: RateLimitStore;
  private cleanupInterval: NodeJS.Timeout;
  
  constructor(
    private options: RateLimitOptions,
    store?: RateLimitStore
  ) {
    this.store = store || new RateLimitStore();
    
    // Cleanup expired entries every minute
    this.cleanupInterval = setInterval(() => {
      this.store.cleanup();
    }, 60000);
  }
  
  async checkLimit(identifier: string): Promise<{
    allowed: boolean;
    info: RateLimitInfo;
  }> {
    const result = await this.store.increment(identifier, this.options.windowMs);
    
    const info: RateLimitInfo = {
      limit: this.options.maxRequests,
      current: result.count,
      remaining: Math.max(0, this.options.maxRequests - result.count),
      resetTime: new Date(result.resetTime)
    };
    
    return {
      allowed: result.count <= this.options.maxRequests,
      info
    };
  }
  
  async isLimited(identifier: string): Promise<boolean> {
    const result = await this.checkLimit(identifier);
    return !result.allowed;
  }
  
  async getRemainingRequests(identifier: string): Promise<number> {
    const result = await this.checkLimit(identifier);
    return result.info.remaining;
  }
  
  destroy(): void {
    if (this.cleanupInterval) {
      clearInterval(this.cleanupInterval);
    }
  }
}

/**
 * Create rate limiter middleware for Fastify
 */
export function createRateLimitMiddleware(options: RateLimitOptions) {
  const limiter = new RateLimiter(options);
  
  return async (request: any, reply: any) => {
    const key = options.keyGenerator 
      ? options.keyGenerator(request)
      : request.ip || 'global';
    
    const result = await limiter.checkLimit(key);
    
    // Add rate limit headers
    reply.header('X-RateLimit-Limit', result.info.limit);
    reply.header('X-RateLimit-Remaining', result.info.remaining);
    reply.header('X-RateLimit-Reset', Math.ceil(result.info.resetTime.getTime() / 1000));
    
    if (!result.allowed) {
      if (options.onLimitReached) {
        options.onLimitReached(request);
      }
      
      reply.status(429).send({
        error: {
          code: 'RATE_LIMIT_EXCEEDED',
          message: 'Too many requests, please try again later.',
          retryAfter: Math.ceil((result.info.resetTime.getTime() - Date.now()) / 1000)
        }
      });
      return;
    }
  };
}

/**
 * Rate limiter for function calls
 */
export function rateLimit<T extends (...args: any[]) => any>(
  fn: T,
  maxCalls: number,
  windowMs: number
): T {
  const calls: number[] = [];
  
  return ((...args: Parameters<T>): ReturnType<T> => {
    const now = Date.now();
    
    // Remove old calls outside the window
    while (calls.length > 0 && calls[0] <= now - windowMs) {
      calls.shift();
    }
    
    if (calls.length >= maxCalls) {
      throw new Error(`Rate limit exceeded: ${maxCalls} calls per ${windowMs}ms`);
    }
    
    calls.push(now);
    return fn(...args);
  }) as T;
}