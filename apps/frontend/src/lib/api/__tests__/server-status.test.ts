import { afterEach, describe, expect, it, vi } from 'vitest'

import { isServerUnavailable, serverStatusStore } from '../server-status'

describe('isServerUnavailable', () => {
  it('returns true for an HTTP 503 error', () => {
    const error = Object.assign(new Error('Service Unavailable'), { status: 503 })
    expect(isServerUnavailable(error)).toBe(true)
  })

  it('returns true for a network TypeError', () => {
    expect(isServerUnavailable(new TypeError('Failed to fetch'))).toBe(true)
  })

  it('returns true for a "Failed to fetch" message on a plain Error', () => {
    expect(isServerUnavailable(new Error('Failed to fetch'))).toBe(true)
  })

  it('returns true for a status 0 (network unreachable) error', () => {
    const error = Object.assign(new Error('network error'), { status: 0 })
    expect(isServerUnavailable(error)).toBe(true)
  })

  it.each([401, 404, 500, 502, 504])('returns false for HTTP %i', (status) => {
    const error = Object.assign(new Error('boom'), { status })
    expect(isServerUnavailable(error)).toBe(false)
  })

  it('returns false for non-error values', () => {
    expect(isServerUnavailable(null)).toBe(false)
    expect(isServerUnavailable(undefined)).toBe(false)
    expect(isServerUnavailable('boom')).toBe(false)
    expect(isServerUnavailable({})).toBe(false)
  })
})

describe('serverStatusStore', () => {
  afterEach(() => {
    serverStatusStore.setUnavailable(false)
  })

  it('defaults to reachable', () => {
    expect(serverStatusStore.getSnapshot()).toBe(false)
  })

  it('notifies subscribers on change, deduplicates, and supports unsubscribe', () => {
    const listener = vi.fn()
    const unsubscribe = serverStatusStore.subscribe(listener)

    serverStatusStore.setUnavailable(true)
    expect(serverStatusStore.getSnapshot()).toBe(true)
    expect(listener).toHaveBeenCalledTimes(1)

    // No notification when the value is unchanged.
    serverStatusStore.setUnavailable(true)
    expect(listener).toHaveBeenCalledTimes(1)

    serverStatusStore.setUnavailable(false)
    expect(serverStatusStore.getSnapshot()).toBe(false)
    expect(listener).toHaveBeenCalledTimes(2)

    // No further notifications after unsubscribing.
    unsubscribe()
    serverStatusStore.setUnavailable(true)
    expect(listener).toHaveBeenCalledTimes(2)
  })
})
