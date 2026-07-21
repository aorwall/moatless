import type React from 'react'
import { useEffect, useEffectEvent, useLayoutEffect, useRef, useState } from 'react'

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

function getScrollElement(scrollRef: React.RefObject<HTMLDivElement | null>): Element | null {
  return scrollRef.current?.querySelector('[data-slot="scroll-area-viewport"]') ?? null
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
  const previousScrollPosition = useRef({
    scrollTop: 0,
    clientHeight: 0,
    scrollHeight: 0,
  })

  // Check if user is at bottom
  const checkIsAtBottom = () => {
    const scrollElement = getScrollElement(scrollRef)
    if (!scrollElement) {
      return false
    }

    const { scrollTop, scrollHeight, clientHeight } = scrollElement
    return Math.abs(scrollHeight - clientHeight - scrollTop) < threshold
  }

  // Scroll to bottom
  const scrollToBottom = () => {
    const scrollElement = getScrollElement(scrollRef)
    if (!scrollElement) {
      return
    }

    requestAnimationFrame(() => {
      scrollElement.scrollTop = scrollElement.scrollHeight
    })
  }

  // Handle scroll events
  const handleScroll = () => {
    const scrollElement = getScrollElement(scrollRef)
    if (!scrollElement) {
      return
    }

    const atBottom = checkIsAtBottom()
    setIsAtBottom(atBottom)

    // Update previousScrollPosition when user manually scrolls
    const { scrollTop, scrollHeight, clientHeight } = scrollElement
    previousScrollPosition.current = { scrollTop, clientHeight, scrollHeight }
  }
  const handleScrollEvent = useEffectEvent(handleScroll)
  const scrollToBottomEvent = useEffectEvent(scrollToBottom)

  // Set up scroll listener
  useLayoutEffect(() => {
    const scrollElement = getScrollElement(scrollRef)
    if (!scrollElement) {
      return
    }

    const onScroll = () => handleScrollEvent()
    scrollElement.addEventListener('scroll', onScroll)
    const initialMeasurementFrame = requestAnimationFrame(onScroll)

    return () => {
      cancelAnimationFrame(initialMeasurementFrame)
      scrollElement.removeEventListener('scroll', onScroll)
    }
  }, [])

  // Handle initial scroll on mount - retry until content is available
  useEffect(() => {
    if (!scrollOnMount || hasScrolledOnMount.current) {
      return
    }

    const tryInitialScroll = () => {
      const scrollElement = getScrollElement(scrollRef)
      if (scrollElement && scrollElement.scrollHeight > scrollElement.clientHeight) {
        scrollToBottomEvent()
        hasScrolledOnMount.current = true
        return true
      }
      return false
    }

    // Try immediately
    if (tryInitialScroll()) {
      return
    }

    // If no content yet, set up an interval to retry
    const intervalId = setInterval(() => {
      if (tryInitialScroll()) {
        clearInterval(intervalId)
      }
    }, 100)

    // Clean up after 5 seconds max
    const timeoutId = setTimeout(() => {
      clearInterval(intervalId)
    }, 5000)

    return () => {
      clearInterval(intervalId)
      clearTimeout(timeoutId)
    }
  }, [scrollOnMount])

  // Auto-scroll when dependencies change if at bottom
  useEffect(() => {
    const scrollElement = getScrollElement(scrollRef)
    if (!scrollElement) {
      return
    }

    const currentScrollHeight = scrollElement.scrollHeight
    const heightIncreased = currentScrollHeight > lastScrollHeight.current

    // Skip if this is the initial mount (handled by scrollOnMount effect)
    if (lastScrollHeight.current === 0) {
      lastScrollHeight.current = currentScrollHeight
      // Initialize previous scroll position
      const { scrollTop, clientHeight, scrollHeight } = scrollElement
      previousScrollPosition.current = {
        scrollTop,
        clientHeight,
        scrollHeight,
      }
      return
    }

    // Check if user WAS at bottom BEFORE this update using previous measurements
    const {
      scrollTop: prevScrollTop,
      clientHeight: prevClientHeight,
      scrollHeight: prevScrollHeight,
    } = previousScrollPosition.current
    const wasAtBottom = Math.abs(prevScrollHeight - prevClientHeight - prevScrollTop) <= threshold

    // Get current scroll position for ref update
    const { scrollTop, scrollHeight, clientHeight } = scrollElement

    // Only auto-scroll if:
    // 1. Content height has increased (new content added)
    // 2. User WAS at the bottom before the update
    if (heightIncreased && wasAtBottom) {
      // Use RAF to ensure DOM has updated
      requestAnimationFrame(() => {
        scrollElement.scrollTop = scrollElement.scrollHeight
      })

      // Update refs to reflect state AFTER auto-scroll (at bottom)
      lastScrollHeight.current = currentScrollHeight
      previousScrollPosition.current = {
        scrollTop: currentScrollHeight - clientHeight,
        clientHeight,
        scrollHeight: currentScrollHeight,
      }
    } else {
      // Update refs with current state
      lastScrollHeight.current = currentScrollHeight
      previousScrollPosition.current = {
        scrollTop,
        clientHeight,
        scrollHeight,
      }
    }
  }, [...dependencies, threshold])

  return {
    scrollRef,
    isAtBottom,
    scrollToBottom,
    handleScroll,
  }
}
