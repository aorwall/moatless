import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { 
  retry, 
  withTimeout, 
  sleep, 
  batchExecute, 
  concurrent,
  debounce,
  throttle,
  makeCancelable,
  memoizeAsync,
  TimeoutError, 
  RetryError 
} from '../async-utils.js';

describe('Async Utils', () => {
  beforeEach(() => {
    vi.clearAllTimers();
    vi.useFakeTimers();
  });
  
  afterEach(() => {
    vi.useRealTimers();
  });
  
  describe('retry', () => {
    it('should succeed on first attempt', async () => {
      const fn = vi.fn().mockResolvedValue('success');
      
      const result = await retry(fn, { maxAttempts: 3 });
      
      expect(result).toBe('success');
      expect(fn).toHaveBeenCalledTimes(1);
    });
    
    it('should retry on failure', async () => {
      const fn = vi.fn()
        .mockRejectedValueOnce(new Error('fail'))
        .mockResolvedValue('success');
      
      const promise = retry(fn, { maxAttempts: 3, baseDelay: 100 });
      
      // Advance timer to trigger retry
      await vi.advanceTimersByTimeAsync(100);
      
      const result = await promise;
      
      expect(result).toBe('success');
      expect(fn).toHaveBeenCalledTimes(2);
    });
    
    it('should throw RetryError after max attempts', async () => {
      const fn = vi.fn().mockRejectedValue(new Error('fail'));
      
      const promise = retry(fn, { maxAttempts: 2, baseDelay: 100 });
      
      // Advance through retries
      await vi.advanceTimersByTimeAsync(200);
      
      await expect(promise).rejects.toThrow(RetryError);
      expect(fn).toHaveBeenCalledTimes(2);
    });
    
    it('should apply exponential backoff', async () => {
      const fn = vi.fn()
        .mockRejectedValueOnce(new Error('fail'))
        .mockRejectedValueOnce(new Error('fail'))
        .mockResolvedValue('success');
      
      const promise = retry(fn, { 
        maxAttempts: 3, 
        baseDelay: 100,
        exponentialBackoff: true 
      });
      
      // First retry after 100ms
      await vi.advanceTimersByTimeAsync(100);
      expect(fn).toHaveBeenCalledTimes(2);
      
      // Second retry after 200ms (exponential)
      await vi.advanceTimersByTimeAsync(200);
      expect(fn).toHaveBeenCalledTimes(3);
      
      const result = await promise;
      expect(result).toBe('success');
    });
    
    it('should call onRetry callback', async () => {
      const fn = vi.fn()
        .mockRejectedValueOnce(new Error('fail'))
        .mockResolvedValue('success');
      
      const onRetry = vi.fn();
      
      const promise = retry(fn, { 
        maxAttempts: 3, 
        baseDelay: 100,
        onRetry 
      });
      
      await vi.advanceTimersByTimeAsync(100);
      await promise;
      
      expect(onRetry).toHaveBeenCalledWith(1, expect.any(Error));
    });
  });
  
  describe('withTimeout', () => {
    it('should resolve if promise completes before timeout', async () => {
      const promise = Promise.resolve('success');
      
      const result = await withTimeout(promise, 1000);
      
      expect(result).toBe('success');
    });
    
    it('should reject with TimeoutError if timeout exceeded', async () => {
      vi.useRealTimers(); // Need real timers for this test
      
      const promise = new Promise(resolve => setTimeout(resolve, 2000));
      
      await expect(withTimeout(promise, 100))
        .rejects.toThrow(TimeoutError);
    });
  });
  
  describe('sleep', () => {
    it('should delay for specified time', async () => {
      const start = Date.now();
      const promise = sleep(1000);
      
      await vi.advanceTimersByTimeAsync(1000);
      await promise;
      
      expect(vi.getTimerCount()).toBe(0);
    });
  });
  
  describe('debounce', () => {
    it('should debounce function calls', () => {
      const fn = vi.fn();
      const debounced = debounce(fn, 100);
      
      debounced();
      debounced();
      debounced();
      
      expect(fn).not.toHaveBeenCalled();
      
      vi.advanceTimersByTime(100);
      
      expect(fn).toHaveBeenCalledTimes(1);
    });
    
    it('should call immediately when immediate is true', () => {
      const fn = vi.fn();
      const debounced = debounce(fn, 100, true);
      
      debounced();
      expect(fn).toHaveBeenCalledTimes(1);
      
      debounced();
      debounced();
      
      vi.advanceTimersByTime(100);
      
      expect(fn).toHaveBeenCalledTimes(1);
    });
  });
  
  describe('throttle', () => {
    it('should throttle function calls', () => {
      const fn = vi.fn();
      const throttled = throttle(fn, 100);
      
      throttled();
      throttled();
      throttled();
      
      expect(fn).toHaveBeenCalledTimes(1);
      
      vi.advanceTimersByTime(100);
      
      throttled();
      expect(fn).toHaveBeenCalledTimes(2);
    });
  });
  
  describe('batchExecute', () => {
    it('should execute items in batches', async () => {
      const items = [1, 2, 3, 4, 5];
      const executor = vi.fn().mockImplementation(x => Promise.resolve(x * 2));
      
      const results = await batchExecute(items, executor, 2);
      
      expect(results).toEqual([2, 4, 6, 8, 10]);
      expect(executor).toHaveBeenCalledTimes(5);
    });
    
    it('should handle errors in batch execution', async () => {
      const items = [1, 2, 3];
      const executor = vi.fn()
        .mockResolvedValueOnce(1)
        .mockRejectedValueOnce(new Error('fail'))
        .mockResolvedValueOnce(3);
      
      await expect(batchExecute(items, executor, 2))
        .rejects.toThrow('fail');
    });
  });
  
  describe('concurrent', () => {
    it('should limit concurrent executions', async () => {
      const items = [1, 2, 3, 4, 5];
      let activeCount = 0;
      let maxActive = 0;
      
      const executor = vi.fn().mockImplementation(async (x) => {
        activeCount++;
        maxActive = Math.max(maxActive, activeCount);
        await new Promise(resolve => setTimeout(resolve, 10));
        activeCount--;
        return x * 2;
      });
      
      const results = await concurrent(items, executor, 2);
      
      expect(results).toEqual([2, 4, 6, 8, 10]);
      expect(maxActive).toBeLessThanOrEqual(2);
    });
  });
  
  describe('makeCancelable', () => {
    it('should create cancelable promise', async () => {
      let resolved = false;
      const promise = new Promise<string>(resolve => {
        setTimeout(() => {
          resolved = true;
          resolve('done');
        }, 100);
      });
      
      const cancelable = makeCancelable(promise);
      
      expect(cancelable.isCanceled()).toBe(false);
      
      cancelable.cancel();
      
      expect(cancelable.isCanceled()).toBe(true);
      
      vi.advanceTimersByTime(100);
      
      // The promise should not resolve after cancellation
      let result = null;
      cancelable.promise.then(val => { result = val; });
      
      await vi.advanceTimersByTimeAsync(100);
      
      expect(result).toBe(null);
    });
  });
  
  describe('memoizeAsync', () => {
    it('should memoize async function results', async () => {
      const fn = vi.fn().mockImplementation(async (x: number) => x * 2);
      const memoized = memoizeAsync(fn);
      
      const result1 = await memoized(5);
      const result2 = await memoized(5);
      const result3 = await memoized(10);
      
      expect(result1).toBe(10);
      expect(result2).toBe(10);
      expect(result3).toBe(20);
      expect(fn).toHaveBeenCalledTimes(2); // Only called for unique arguments
    });
    
    it('should respect TTL', async () => {
      vi.useRealTimers(); // Need real timers for TTL
      
      const fn = vi.fn().mockImplementation(async (x: number) => x * 2);
      const memoized = memoizeAsync(fn, undefined, 100); // 100ms TTL
      
      const result1 = await memoized(5);
      expect(fn).toHaveBeenCalledTimes(1);
      
      // Within TTL
      const result2 = await memoized(5);
      expect(fn).toHaveBeenCalledTimes(1);
      
      // Wait for TTL to expire
      await new Promise(resolve => setTimeout(resolve, 150));
      
      const result3 = await memoized(5);
      expect(fn).toHaveBeenCalledTimes(2);
      
      expect(result1).toBe(10);
      expect(result2).toBe(10);
      expect(result3).toBe(10);
    });
  });
});