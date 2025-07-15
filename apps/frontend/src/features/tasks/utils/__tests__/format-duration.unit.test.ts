import { describe, expect, it, vi } from 'vitest'
import { formatRunningDuration } from '../format-duration'

describe('formatRunningDuration', () => {
  it('should format duration correctly with Date object', () => {
    const startTime = new Date('2024-01-01T10:00:00Z')

    // Mock Date constructor to return fixed 'now' time
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2024-01-01T10:02:30Z'))

    const result = formatRunningDuration(startTime)
    expect(result).toBe('2m 30s')

    vi.useRealTimers()
  })

  it('should format duration correctly with ISO string', () => {
    const startTime = '2024-01-01T10:00:00Z'

    // Mock Date constructor to return fixed 'now' time
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2024-01-01T10:01:45Z'))

    const result = formatRunningDuration(startTime)
    expect(result).toBe('1m 45s')

    vi.useRealTimers()
  })

  it('should handle seconds-only duration', () => {
    const startTime = new Date('2024-01-01T10:00:00Z')

    // Mock Date constructor to return fixed 'now' time
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2024-01-01T10:00:45Z'))

    const result = formatRunningDuration(startTime)
    expect(result).toBe('45s')

    vi.useRealTimers()
  })

  it('should handle invalid date string gracefully', () => {
    const result = formatRunningDuration('invalid-date')
    expect(result).toBe('0s')
  })

  it('should handle negative duration gracefully', () => {
    const futureTime = new Date('2024-01-01T10:00:00Z')

    // Mock Date constructor to return past time
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2024-01-01T09:59:00Z'))

    const result = formatRunningDuration(futureTime)
    expect(result).toBe('0s')

    vi.useRealTimers()
  })
})
