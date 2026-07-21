import { afterEach, describe, expect, it, vi } from 'vitest'
import { getCurrentPath } from '../client/route-tracker'

function stubLocation(loc: { pathname: string; search?: string; hash?: string }): void {
  vi.stubGlobal('window', {
    location: { pathname: loc.pathname, search: loc.search ?? '', hash: loc.hash ?? '' },
  })
}

describe('getCurrentPath', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('returns pathname + search for a normal route', () => {
    stubLocation({ pathname: '/products', search: '?category=shoes' })
    expect(getCurrentPath()).toBe('/products?category=shoes')
  })

  it('prefers the hash route when the hash starts with #/', () => {
    stubLocation({ pathname: '/', hash: '#/dashboard' })
    expect(getCurrentPath()).toBe('#/dashboard')
  })

  it('drops the query and reports pathname alone when the URL is pathologically long', () => {
    // Mimics a self-preview loop: the host has nested this app's URL back into
    // its own `?path=`, producing a multi-KB query that must not be propagated.
    const hugeSearch = `?path=${'%252F'.repeat(1000)}`
    stubLocation({ pathname: '/tasks/abc', search: hugeSearch })
    expect(getCurrentPath()).toBe('/tasks/abc')
  })
})
