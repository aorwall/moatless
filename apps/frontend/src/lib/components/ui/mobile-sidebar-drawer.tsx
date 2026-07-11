import * as React from 'react'
import { cn } from '@/lib/utils'

/** Distance (px) from the left edge where an opening swipe must start. */
const EDGE_SIZE = 24
/** Minimum travel (px) before a gesture locks into horizontal vs vertical. */
const DIRECTION_LOCK = 8
/** Fallback panel width (px) used before the element can be measured (20rem). */
const FALLBACK_WIDTH = 320

interface MobileSidebarDrawerProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  children: React.ReactNode
  className?: string | undefined
}

/**
 * Mobile task-list drawer that the user can drag in from the left edge with a
 * finger (and drag back out to dismiss), instead of the swipe triggering the
 * browser's back navigation. The panel follows the finger 1:1 and snaps
 * open/closed on release. Driven by the shared `openMobile` sidebar state, so
 * the hamburger trigger still opens it (animated).
 *
 * Must be rendered within a `SidebarProvider`.
 */
export function MobileSidebarDrawer({
  open: openMobile,
  onOpenChange: setOpenMobile,
  children,
  className,
}: MobileSidebarDrawerProps) {
  const panelRef = React.useRef<HTMLDivElement>(null)

  // Inline px offset applied only while dragging; when null the panel rests at
  // its open/closed position via CSS transition.
  const [dragX, setDragX] = React.useState<number | null>(null)
  const [isDragging, setIsDragging] = React.useState(false)

  // Latest open state, read from inside the (stable) gesture handlers.
  const openRef = React.useRef(openMobile)
  openRef.current = openMobile

  React.useEffect(() => {
    const gesture = {
      active: false,
      decided: false,
      horizontal: false,
      startX: 0,
      startY: 0,
      base: 0,
      width: FALLBACK_WIDTH,
      lastX: 0,
    }

    const measure = () => panelRef.current?.offsetWidth || FALLBACK_WIDTH

    const handleTouchStart = (event: TouchEvent) => {
      if (event.touches.length !== 1) {
        gesture.active = false
        return
      }
      const touch = event.touches[0]
      if (!touch) {
        return
      }
      const width = measure()
      if (openRef.current) {
        // Drawer is open: allow a drag-to-close from anywhere over it.
        gesture.active = true
        gesture.base = 0
      } else if (touch.clientX <= EDGE_SIZE) {
        // Drawer is closed: only start when the finger lands on the left edge.
        // Claim the gesture up front so an edge swipe drags the drawer rather
        // than starting the browser's overscroll back-navigation. (Note: iOS
        // Safari / Android system edge-back gestures are OS-level and can't be
        // fully suppressed from JS — this is best-effort.)
        gesture.active = true
        gesture.base = -width
        event.preventDefault()
      } else {
        gesture.active = false
        return
      }
      gesture.decided = false
      gesture.horizontal = false
      gesture.startX = touch.clientX
      gesture.startY = touch.clientY
      gesture.width = width
      gesture.lastX = gesture.base
    }

    const handleTouchMove = (event: TouchEvent) => {
      if (!gesture.active) {
        return
      }
      const touch = event.touches[0]
      if (!touch) {
        return
      }
      const deltaX = touch.clientX - gesture.startX
      const deltaY = touch.clientY - gesture.startY

      if (!gesture.decided) {
        if (Math.abs(deltaX) < DIRECTION_LOCK && Math.abs(deltaY) < DIRECTION_LOCK) {
          return
        }
        gesture.decided = true
        gesture.horizontal = Math.abs(deltaX) > Math.abs(deltaY)
        if (!gesture.horizontal) {
          // Vertical gesture — release so the list can scroll.
          gesture.active = false
          return
        }
        setIsDragging(true)
      }

      // Block the browser back-swipe / page scroll while we own the gesture.
      event.preventDefault()
      const next = Math.min(0, Math.max(-gesture.width, gesture.base + deltaX))
      gesture.lastX = next
      setDragX(next)
    }

    const handleTouchEnd = () => {
      if (gesture.active && gesture.horizontal) {
        // Snap based on how far past the halfway point the panel was dragged.
        setOpenMobile(gesture.lastX > -gesture.width / 2)
      }
      gesture.active = false
      gesture.decided = false
      gesture.horizontal = false
      setIsDragging(false)
      setDragX(null)
    }

    document.addEventListener('touchstart', handleTouchStart, { passive: false })
    document.addEventListener('touchmove', handleTouchMove, { passive: false })
    document.addEventListener('touchend', handleTouchEnd)
    document.addEventListener('touchcancel', handleTouchEnd)
    return () => {
      document.removeEventListener('touchstart', handleTouchStart)
      document.removeEventListener('touchmove', handleTouchMove)
      document.removeEventListener('touchend', handleTouchEnd)
      document.removeEventListener('touchcancel', handleTouchEnd)
    }
  }, [setOpenMobile])

  // Close on Escape while open.
  React.useEffect(() => {
    if (!openMobile) {
      return
    }
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpenMobile(false)
      }
    }
    document.addEventListener('keydown', handleKey)
    return () => document.removeEventListener('keydown', handleKey)
  }, [openMobile, setOpenMobile])

  // Progress 0 (closed) → 1 (open), for the backdrop opacity while dragging.
  const width = panelRef.current?.offsetWidth || FALLBACK_WIDTH
  const progress = dragX === null ? (openMobile ? 1 : 0) : (dragX + width) / width
  const showOverlay = openMobile || isDragging

  return (
    <>
      {/** biome-ignore lint/a11y/noStaticElementInteractions: backdrop click-to-dismiss mirrors dialog overlay behaviour */}
      <div
        aria-hidden="true"
        onClick={() => setOpenMobile(false)}
        className={cn(
          'fixed inset-0 z-40 bg-black/50',
          isDragging ? 'transition-none' : 'transition-opacity duration-300',
          showOverlay ? 'pointer-events-auto' : 'pointer-events-none',
        )}
        style={{ opacity: showOverlay ? Math.max(0, Math.min(1, progress)) : 0 }}
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label="Task list"
        className={cn(
          'fixed inset-y-0 left-0 z-50 flex h-full w-80 max-w-[85vw] flex-col overflow-x-hidden overscroll-x-none border-r bg-sidebar text-sidebar-foreground',
          isDragging ? 'transition-none' : 'transition-transform duration-300 ease-in-out',
          dragX === null && (openMobile ? 'translate-x-0' : '-translate-x-full'),
          !(openMobile || isDragging) && 'pointer-events-none',
          className,
        )}
        style={dragX === null ? undefined : { transform: `translate3d(${dragX}px, 0, 0)` }}
      >
        {children}
      </div>
    </>
  )
}
