import { useEffect } from 'react'

interface EdgeSwipeOptions {
  /** Whether the gesture listener is active. */
  enabled?: boolean
  /** Distance (px) from the left edge where a swipe must start. */
  edgeSize?: number
  /** Horizontal distance (px) the finger must travel to trigger. */
  threshold?: number
  /** Called when a qualifying left-edge → right swipe completes. */
  onSwipeRight: () => void
}

/**
 * Detects a swipe that starts near the left edge of the screen and moves
 * rightward, used to open a left-anchored drawer/sidebar on touch devices.
 *
 * The gesture is only recognised when it begins within `edgeSize` px of the
 * left edge and travels more horizontally than vertically, so it doesn't fight
 * with vertical scrolling inside the page.
 */
export function useEdgeSwipe({
  enabled = true,
  edgeSize = 24,
  threshold = 60,
  onSwipeRight,
}: EdgeSwipeOptions) {
  useEffect(() => {
    if (!enabled) {
      return
    }

    let startX = 0
    let startY = 0
    let tracking = false

    const handleTouchStart = (event: TouchEvent) => {
      // Ignore multi-touch (pinch/zoom) gestures.
      if (event.touches.length !== 1) {
        tracking = false
        return
      }
      const touch = event.touches[0]
      if (touch.clientX <= edgeSize) {
        startX = touch.clientX
        startY = touch.clientY
        tracking = true
      } else {
        tracking = false
      }
    }

    const handleTouchMove = (event: TouchEvent) => {
      if (!tracking) {
        return
      }
      const touch = event.touches[0]
      if (!touch) {
        return
      }
      const deltaX = touch.clientX - startX
      const deltaY = touch.clientY - startY
      if (deltaX > threshold && deltaX > Math.abs(deltaY)) {
        tracking = false
        onSwipeRight()
      }
    }

    const stopTracking = () => {
      tracking = false
    }

    window.addEventListener('touchstart', handleTouchStart, { passive: true })
    window.addEventListener('touchmove', handleTouchMove, { passive: true })
    window.addEventListener('touchend', stopTracking, { passive: true })
    window.addEventListener('touchcancel', stopTracking, { passive: true })

    return () => {
      window.removeEventListener('touchstart', handleTouchStart)
      window.removeEventListener('touchmove', handleTouchMove)
      window.removeEventListener('touchend', stopTracking)
      window.removeEventListener('touchcancel', stopTracking)
    }
  }, [enabled, edgeSize, threshold, onSwipeRight])
}
