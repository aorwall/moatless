import { useCallback, useEffect, useRef, useState } from 'react'

interface UseAutoScrollOptions {
  /** Threshold in pixels to consider user "at bottom" */
  threshold?: number
  /** Whether to scroll to bottom on initial mount */
  scrollOnMount?: boolean
  /** Dependency array to trigger scroll checks */
  dependencies?: unknown[]
}

interface UseAutoScrollResult {
  scrollRef: React.RefObject<HTMLDivElement | null>
  isAtBottom: boolean
  scrollToBottom: () => void
  handleScroll: () => void
}

/**
 * Hook to manage auto-scrolling behavior for chat-like interfaces.
 * Maintains scroll position when new content arrives unless user is at the bottom.
 */
export function useAutoScroll({
  threshold = 10,
  scrollOnMount = true,
  dependencies = [],
}: UseAutoScrollOptions = {}): UseAutoScrollResult {
  const scrollRef = useRef<HTMLDivElement>(null)
  const [isAtBottom, setIsAtBottom] = useState(true)
  const hasScrolledOnMount = useRef(false)
  const lastScrollHeight = useRef(0)

  // Get the actual scrollable element
  const getScrollElement = useCallback(() => {
    if (!scrollRef.current) return null
    return scrollRef.current.querySelector('[data-slot="scroll-area-viewport"]')
  }, [])

  // Check if user is at bottom
  const checkIsAtBottom = useCallback(() => {
    const scrollElement = getScrollElement()
    if (!scrollElement) return false

    const { scrollTop, scrollHeight, clientHeight } = scrollElement
    return Math.abs(scrollHeight - clientHeight - scrollTop) < threshold
  }, [getScrollElement, threshold])

  // Scroll to bottom
  const scrollToBottom = useCallback(() => {
    const scrollElement = getScrollElement()
    if (!scrollElement) return

    requestAnimationFrame(() => {
      scrollElement.scrollTop = scrollElement.scrollHeight
    })
  }, [getScrollElement])

  // Handle scroll events
  const handleScroll = useCallback(() => {
    const atBottom = checkIsAtBottom()
    setIsAtBottom(atBottom)
  }, [checkIsAtBottom])

  // Set up scroll listener
  useEffect(() => {
    const scrollElement = getScrollElement()
    if (!scrollElement) return

    scrollElement.addEventListener('scroll', handleScroll)
    handleScroll() // Check initial state

    return () => {
      scrollElement.removeEventListener('scroll', handleScroll)
    }
  }, [getScrollElement, handleScroll])

  // Handle initial scroll on mount
  useEffect(() => {
    if (scrollOnMount && !hasScrolledOnMount.current) {
      const scrollElement = getScrollElement()
      if (scrollElement && scrollElement.scrollHeight > 0) {
        scrollToBottom()
        hasScrolledOnMount.current = true
      }
    }
  }, [scrollOnMount, getScrollElement, scrollToBottom])

  // Auto-scroll when dependencies change if at bottom
  useEffect(() => {
    const scrollElement = getScrollElement()
    if (!scrollElement) return

    const currentScrollHeight = scrollElement.scrollHeight

    // Only auto-scroll if:
    // 1. Content height has increased (new content added)
    // 2. User was at the bottom before the update
    if (currentScrollHeight > lastScrollHeight.current && isAtBottom) {
      // Save current scroll position
      const currentScrollTop = scrollElement.scrollTop

      // Use RAF to ensure DOM has updated
      requestAnimationFrame(() => {
        // Double-check we're still at bottom (in case user scrolled during update)
        const { scrollTop, scrollHeight, clientHeight } = scrollElement
        const stillAtBottom = Math.abs(scrollHeight - clientHeight - scrollTop) < threshold

        if (stillAtBottom || scrollTop === currentScrollTop) {
          scrollElement.scrollTop = scrollElement.scrollHeight
        }
      })
    }

    lastScrollHeight.current = currentScrollHeight
  }, [...dependencies, isAtBottom, getScrollElement, threshold])

  return {
    scrollRef,
    isAtBottom,
    scrollToBottom,
    handleScroll,
  }
}
