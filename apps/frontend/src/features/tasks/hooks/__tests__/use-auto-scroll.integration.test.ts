import { renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useAutoScroll } from '../use-auto-scroll'

/**
 * Integration tests for useAutoScroll hook
 *
 * Note: Full scroll behavior testing is better done via Storybook
 * where you can visually verify auto-scroll with real DOM interactions.
 * See: apps/frontend/src/lib/components/chat/message-history/__stories__/message-history.stories.tsx
 * - AutoScrollBehavior story
 *
 * These tests verify the hook's basic API and initialization.
 */
describe('useAutoScroll - Integration Tests', () => {
  it('should initialize with correct default values', () => {
    const { result } = renderHook(() => useAutoScroll({ scrollOnMount: false }))

    expect(result.current.scrollRef).toBeDefined()
    expect(result.current.isAtBottom).toBe(true)
    expect(typeof result.current.scrollToBottom).toBe('function')
    expect(typeof result.current.handleScroll).toBe('function')
  })

  it('should accept custom threshold option', () => {
    const { result } = renderHook(() =>
      useAutoScroll({
        threshold: 50,
        scrollOnMount: false,
      }),
    )

    expect(result.current.scrollRef).toBeDefined()
    expect(result.current.isAtBottom).toBe(true)
  })

  it('should accept dependencies array', () => {
    const { result } = renderHook(() =>
      useAutoScroll({
        dependencies: ['dep1', 'dep2'],
        scrollOnMount: false,
      }),
    )

    expect(result.current).toBeDefined()
  })

  it('should have stable function references', () => {
    const { result, rerender } = renderHook(() => useAutoScroll({ scrollOnMount: false }))

    const initialScrollToBottom = result.current.scrollToBottom
    const initialHandleScroll = result.current.handleScroll

    rerender()

    // Functions should be stable across renders (memoized)
    expect(result.current.scrollToBottom).toBe(initialScrollToBottom)
    expect(result.current.handleScroll).toBe(initialHandleScroll)
  })

  it('should handle scrollOnMount option', () => {
    const { result: resultWithScroll } = renderHook(() => useAutoScroll({ scrollOnMount: true }))
    const { result: resultWithoutScroll } = renderHook(() =>
      useAutoScroll({ scrollOnMount: false }),
    )

    // Both should initialize successfully
    expect(resultWithScroll.current).toBeDefined()
    expect(resultWithoutScroll.current).toBeDefined()
  })
})
