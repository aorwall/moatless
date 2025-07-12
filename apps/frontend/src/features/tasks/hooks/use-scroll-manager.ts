import { useCallback, useEffect, useRef, useState } from 'react'

interface UseScrollManagerProps {
  scrollRef: React.RefObject<HTMLDivElement | null>
  bottomRef: React.RefObject<HTMLDivElement | null>
}

export function useScrollManager({ scrollRef, bottomRef }: UseScrollManagerProps) {
  // Use ref to track scroll state without causing re-renders
  const scrollMetricsRef = useRef({ scrollTop: 0, scrollHeight: 0, clientHeight: 0 })
  const [isAtBottom, setIsAtBottom] = useState(true)

  // Handle scroll to bottom button click
  const scrollToBottom = useCallback(() => {
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [bottomRef])

  // Optimized scroll handler to only update state when isAtBottom changes
  useEffect(() => {
    const scrollArea = scrollRef.current
    if (!scrollArea) return

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return

    let rafId: number | null = null

    const handleScroll = () => {
      if (rafId) return

      rafId = requestAnimationFrame(() => {
        const scrollTop = viewport.scrollTop
        const scrollHeight = viewport.scrollHeight
        const clientHeight = viewport.clientHeight

        // Update ref values (no re-render)
        scrollMetricsRef.current = { scrollTop, scrollHeight, clientHeight }

        // Only update state when isAtBottom status changes
        const newIsAtBottom = scrollHeight - scrollTop - clientHeight < 50
        if (newIsAtBottom !== isAtBottom) {
          setIsAtBottom(newIsAtBottom)
        }

        rafId = null
      })
    }

    // Initial values
    const initialScrollTop = viewport.scrollTop
    const initialScrollHeight = viewport.scrollHeight
    const initialClientHeight = viewport.clientHeight
    scrollMetricsRef.current = {
      scrollTop: initialScrollTop,
      scrollHeight: initialScrollHeight,
      clientHeight: initialClientHeight,
    }

    const initialIsAtBottom = initialScrollHeight - initialScrollTop - initialClientHeight < 50
    setIsAtBottom(initialIsAtBottom)

    viewport.addEventListener('scroll', handleScroll)

    // Also observe for size changes
    const resizeObserver = new ResizeObserver(() => {
      const scrollTop = viewport.scrollTop
      const scrollHeight = viewport.scrollHeight
      const clientHeight = viewport.clientHeight

      scrollMetricsRef.current = { scrollTop, scrollHeight, clientHeight }

      const newIsAtBottom = scrollHeight - scrollTop - clientHeight < 50
      if (newIsAtBottom !== isAtBottom) {
        setIsAtBottom(newIsAtBottom)
      }
    })
    resizeObserver.observe(viewport)

    return () => {
      viewport.removeEventListener('scroll', handleScroll)
      resizeObserver.disconnect()
      if (rafId) cancelAnimationFrame(rafId)
    }
  }, [scrollRef, isAtBottom])

  // Scroll to bottom when component mounts (initial load)
  useEffect(() => {
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [bottomRef]) // Only on mount

  // Scroll to bottom when user is already at bottom and content changes
  useEffect(() => {
    if (bottomRef.current && isAtBottom) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [isAtBottom, bottomRef])

  return {
    isAtBottom,
    scrollToBottom,
  }
}
