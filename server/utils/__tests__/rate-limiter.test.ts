import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { 
  RateLimiter, 
  createRateLimitMiddleware, 
  rateLimit 
} from '../rate-limiter.js';

describe('RateLimiter', () => {
  let limiter: RateLimiter;
  
  beforeEach(() => {
    vi.useFakeTimers();
  });
  
  afterEach(() => {
    vi.useRealTimers();
    if (limiter) {
      limiter.destroy();
    }
  });
  
  describe('basic rate limiting', () => {
    it('should allow requests within limit', async () => {
      limiter = new RateLimiter({
        windowMs: 60000, // 1 minute
        maxRequests: 5
      });
      
      for (let i = 0; i < 5; i++) {
        const result = await limiter.checkLimit('user1');
        expect(result.allowed).toBe(true);
        expect(result.info.remaining).toBe(4 - i);
      }
    });
    
    it('should block requests exceeding limit', async () => {
      limiter = new RateLimiter({
        windowMs: 60000,
        maxRequests: 2
      });
      
      await limiter.checkLimit('user1');
      await limiter.checkLimit('user1');
      
      const result = await limiter.checkLimit('user1');
      expect(result.allowed).toBe(false);
      expect(result.info.remaining).toBe(0);
    });
    
    it('should reset after window expires', async () => {
      limiter = new RateLimiter({
        windowMs: 60000,
        maxRequests: 2
      });
      
      await limiter.checkLimit('user1');
      await limiter.checkLimit('user1');
      
      // Advance time past window
      vi.advanceTimersByTime(61000);
      
      const result = await limiter.checkLimit('user1');
      expect(result.allowed).toBe(true);
      expect(result.info.remaining).toBe(1);
    });
    
    it('should track different identifiers separately', async () => {
      limiter = new RateLimiter({
        windowMs: 60000,
        maxRequests: 1
      });
      
      const result1 = await limiter.checkLimit('user1');
      const result2 = await limiter.checkLimit('user2');
      
      expect(result1.allowed).toBe(true);
      expect(result2.allowed).toBe(true);
      
      const result3 = await limiter.checkLimit('user1');
      expect(result3.allowed).toBe(false);
    });
  });
  
  describe('helper methods', () => {
    it('should check if identifier is limited', async () => {
      limiter = new RateLimiter({
        windowMs: 60000,
        maxRequests: 1
      });
      
      expect(await limiter.isLimited('user1')).toBe(false);
      
      await limiter.checkLimit('user1');
      
      expect(await limiter.isLimited('user1')).toBe(true);
    });
    
    it('should get remaining requests', async () => {
      limiter = new RateLimiter({
        windowMs: 60000,
        maxRequests: 3
      });
      
      expect(await limiter.getRemainingRequests('user1')).toBe(3);
      
      await limiter.checkLimit('user1');
      
      expect(await limiter.getRemainingRequests('user1')).toBe(1);
    });
  });
  
  describe('cleanup', () => {
    it('should clean up expired entries', async () => {
      limiter = new RateLimiter({
        windowMs: 1000,
        maxRequests: 1
      });
      
      await limiter.checkLimit('user1');
      await limiter.checkLimit('user2');
      
      // Advance time to trigger cleanup
      vi.advanceTimersByTime(61000);
      
      // Entries should be fresh after cleanup
      const result1 = await limiter.checkLimit('user1');
      const result2 = await limiter.checkLimit('user2');
      
      expect(result1.allowed).toBe(true);
      expect(result2.allowed).toBe(true);
    });
  });
});

describe('createRateLimitMiddleware', () => {
  it('should create middleware that limits requests', async () => {
    const middleware = createRateLimitMiddleware({
      windowMs: 60000,
      maxRequests: 2
    });
    
    const req = { ip: '127.0.0.1' };
    const reply = {
      header: vi.fn().mockReturnThis(),
      status: vi.fn().mockReturnThis(),
      send: vi.fn()
    };
    
    // First two requests should pass
    await middleware(req, reply);
    expect(reply.status).not.toHaveBeenCalled();
    
    await middleware(req, reply);
    expect(reply.status).not.toHaveBeenCalled();
    
    // Third request should be rate limited
    await middleware(req, reply);
    expect(reply.status).toHaveBeenCalledWith(429);
    expect(reply.send).toHaveBeenCalledWith({
      error: {
        code: 'RATE_LIMIT_EXCEEDED',
        message: 'Too many requests, please try again later.',
        retryAfter: expect.any(Number)
      }
    });
  });
  
  it('should add rate limit headers', async () => {
    const middleware = createRateLimitMiddleware({
      windowMs: 60000,
      maxRequests: 5
    });
    
    const req = { ip: '127.0.0.1' };
    const reply = {
      header: vi.fn().mockReturnThis(),
      status: vi.fn().mockReturnThis(),
      send: vi.fn()
    };
    
    await middleware(req, reply);
    
    expect(reply.header).toHaveBeenCalledWith('X-RateLimit-Limit', 5);
    expect(reply.header).toHaveBeenCalledWith('X-RateLimit-Remaining', 4);
    expect(reply.header).toHaveBeenCalledWith('X-RateLimit-Reset', expect.any(Number));
  });
  
  it('should use custom key generator', async () => {
    const keyGenerator = vi.fn((req) => req.userId);
    const middleware = createRateLimitMiddleware({
      windowMs: 60000,
      maxRequests: 1,
      keyGenerator
    });
    
    const req1 = { ip: '127.0.0.1', userId: 'user1' };
    const req2 = { ip: '127.0.0.1', userId: 'user2' };
    const reply = {
      header: vi.fn().mockReturnThis(),
      status: vi.fn().mockReturnThis(),
      send: vi.fn()
    };
    
    await middleware(req1, reply);
    await middleware(req2, reply);
    
    expect(keyGenerator).toHaveBeenCalledWith(req1);
    expect(keyGenerator).toHaveBeenCalledWith(req2);
    expect(reply.status).not.toHaveBeenCalled();
  });
  
  it('should call onLimitReached callback', async () => {
    const onLimitReached = vi.fn();
    const middleware = createRateLimitMiddleware({
      windowMs: 60000,
      maxRequests: 1,
      onLimitReached
    });
    
    const req = { ip: '127.0.0.1' };
    const reply = {
      header: vi.fn().mockReturnThis(),
      status: vi.fn().mockReturnThis(),
      send: vi.fn()
    };
    
    await middleware(req, reply);
    await middleware(req, reply);
    
    expect(onLimitReached).toHaveBeenCalledWith(req);
  });
});

describe('rateLimit function wrapper', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  
  afterEach(() => {
    vi.useRealTimers();
  });
  
  it('should limit function calls', () => {
    const fn = vi.fn();
    const limited = rateLimit(fn, 2, 1000);
    
    limited();
    limited();
    
    expect(fn).toHaveBeenCalledTimes(2);
    
    expect(() => limited()).toThrow('Rate limit exceeded');
    expect(fn).toHaveBeenCalledTimes(2);
  });
  
  it('should reset after window', () => {
    const fn = vi.fn();
    const limited = rateLimit(fn, 1, 1000);
    
    limited();
    expect(() => limited()).toThrow();
    
    vi.advanceTimersByTime(1001);
    
    limited();
    expect(fn).toHaveBeenCalledTimes(2);
  });
  
  it('should preserve function return value', () => {
    const fn = vi.fn().mockReturnValue('result');
    const limited = rateLimit(fn, 10, 1000);
    
    const result = limited();
    
    expect(result).toBe('result');
  });
});