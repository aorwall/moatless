import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { queryClient } from '../query-client'
import { serverStatusStore } from '../server-status'

/**
 * These tests lock in the scoping rule: the full-page `ServerDownGate` (driven
 * by `serverStatusStore`) must only be flipped by the auth-session probe
 * (`meta: { serverHealthProbe: true }`). A 503 from any other endpoint — e.g. a
 * sandbox files/tree fetch — must fail only its own query, not the whole app.
 */
describe('queryClient server-status scoping', () => {
  function serviceUnavailable(): Error {
    return Object.assign(new Error('Service Unavailable'), { status: 503 })
  }

  beforeEach(() => {
    serverStatusStore.setUnavailable(false)
    queryClient.clear()
  })

  afterEach(() => {
    serverStatusStore.setUnavailable(false)
    queryClient.clear()
  })

  it('does NOT flag the server down when a non-probe query returns 503', async () => {
    await queryClient
      .fetchQuery({
        queryKey: ['files', 'tree'],
        queryFn: () => Promise.reject(serviceUnavailable()),
        retry: false,
      })
      .catch(() => {
        /* expected rejection */
      })

    expect(serverStatusStore.getSnapshot()).toBe(false)
  })

  it('flags the server down when the auth-session probe returns 503', async () => {
    await queryClient
      .fetchQuery({
        queryKey: ['auth', 'session'],
        queryFn: () => Promise.reject(serviceUnavailable()),
        retry: false,
        meta: { serverHealthProbe: true },
      })
      .catch(() => {
        /* expected rejection */
      })

    expect(serverStatusStore.getSnapshot()).toBe(true)
  })

  it('clears the flag when the probe succeeds again', async () => {
    serverStatusStore.setUnavailable(true)

    await queryClient.fetchQuery({
      queryKey: ['auth', 'session'],
      queryFn: () => Promise.resolve({ ok: true }),
      meta: { serverHealthProbe: true },
    })

    expect(serverStatusStore.getSnapshot()).toBe(false)
  })

  it('does NOT clear a genuine server-down flag when a non-probe query succeeds', async () => {
    serverStatusStore.setUnavailable(true)

    await queryClient.fetchQuery({
      queryKey: ['files', 'tree'],
      queryFn: () => Promise.resolve({ ok: true }),
    })

    expect(serverStatusStore.getSnapshot()).toBe(true)
  })
})
