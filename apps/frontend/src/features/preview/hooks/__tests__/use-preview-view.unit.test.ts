import { renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { usePreviewPath } from '../use-preview-view'

type NavigateOpts = { search: (prev: Record<string, unknown>) => Record<string, unknown> }
const navigateMock = vi.fn((_opts: NavigateOpts) => Promise.resolve())

vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => navigateMock,
  useSearch: () => ({}),
}))

describe('usePreviewPath.setPreviewPath', () => {
  beforeEach(() => {
    navigateMock.mockClear()
  })

  it('writes a normal preview path (with query) to the URL', () => {
    const { result } = renderHook(() => usePreviewPath())

    result.current.setPreviewPath('/products?category=shoes')

    expect(navigateMock).toHaveBeenCalledTimes(1)
    const arg = navigateMock.mock.calls[0]![0]
    expect(arg.search({ device: 'desktop' })).toEqual({
      device: 'desktop',
      path: '/products?category=shoes',
    })
  })

  it('ignores a pathologically long path (self-preview recursion guard)', () => {
    const { result } = renderHook(() => usePreviewPath())
    const huge = `/tasks/x?path=${'%252F'.repeat(1000)}`
    expect(huge.length).toBeGreaterThan(2048)

    result.current.setPreviewPath(huge)

    expect(navigateMock).not.toHaveBeenCalled()
  })

  it('accepts a path at the length cap boundary', () => {
    const { result } = renderHook(() => usePreviewPath())
    const atCap = '/'.padEnd(2048, 'a')
    expect(atCap.length).toBe(2048)

    result.current.setPreviewPath(atCap)

    expect(navigateMock).toHaveBeenCalledTimes(1)
  })
})
