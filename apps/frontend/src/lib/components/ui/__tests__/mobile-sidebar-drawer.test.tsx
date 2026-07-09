import { act, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MobileSidebarDrawer } from '@/lib/components/ui/mobile-sidebar-drawer'

/**
 * The drawer registers its gesture handlers on `document`, so these tests
 * dispatch raw touch events there. jsdom has no layout, so `offsetWidth` is 0
 * and the drawer falls back to its 288px width — meaning the halfway snap
 * point sits at -144px.
 */

type Point = { clientX: number; clientY: number }

function dispatchTouch(type: string, points: Point[]) {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'touches', { value: points, configurable: true })
  act(() => {
    document.dispatchEvent(event)
  })
  return event
}

function renderDrawer(open: boolean) {
  const onOpenChange = vi.fn()
  render(
    <MobileSidebarDrawer open={open} onOpenChange={onOpenChange}>
      <div>Task list content</div>
    </MobileSidebarDrawer>,
  )
  const panel = screen.getByRole('dialog', { name: 'Task list' })
  return { onOpenChange, panel }
}

describe('MobileSidebarDrawer', () => {
  it('opens when swiped in from the left edge past the halfway point', () => {
    const { onOpenChange } = renderDrawer(false)

    dispatchTouch('touchstart', [{ clientX: 5, clientY: 100 }])
    dispatchTouch('touchmove', [{ clientX: 250, clientY: 110 }]) // +245px, well past -144
    dispatchTouch('touchend', [])

    expect(onOpenChange).toHaveBeenCalledWith(true)
  })

  it('stays closed when the swipe does not cross the halfway point', () => {
    const { onOpenChange } = renderDrawer(false)

    dispatchTouch('touchstart', [{ clientX: 5, clientY: 100 }])
    dispatchTouch('touchmove', [{ clientX: 80, clientY: 105 }]) // ends at -213px, short of -144
    dispatchTouch('touchend', [])

    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('ignores touches that do not start at the left edge while closed', () => {
    const { onOpenChange } = renderDrawer(false)

    dispatchTouch('touchstart', [{ clientX: 200, clientY: 100 }])
    dispatchTouch('touchmove', [{ clientX: 400, clientY: 110 }])
    dispatchTouch('touchend', [])

    expect(onOpenChange).not.toHaveBeenCalled()
  })

  it('does not hijack a vertical scroll gesture', () => {
    const { onOpenChange } = renderDrawer(false)

    dispatchTouch('touchstart', [{ clientX: 5, clientY: 100 }])
    const move = dispatchTouch('touchmove', [{ clientX: 12, clientY: 260 }]) // mostly vertical
    dispatchTouch('touchend', [])

    expect(onOpenChange).not.toHaveBeenCalled()
    expect(move.defaultPrevented).toBe(false)
  })

  it('prevents default on an edge start and horizontal drag to suppress page navigation', () => {
    renderDrawer(false)

    // Claiming the gesture at the edge helps stop overscroll back-navigation.
    // (OS-level edge-back gestures still cannot be fully suppressed from JS.)
    const start = dispatchTouch('touchstart', [{ clientX: 5, clientY: 100 }])
    const move = dispatchTouch('touchmove', [{ clientX: 120, clientY: 105 }])

    expect(start.defaultPrevented).toBe(true)
    expect(move.defaultPrevented).toBe(true)
  })

  it('follows the finger by translating the panel while dragging', () => {
    const { panel } = renderDrawer(false)

    dispatchTouch('touchstart', [{ clientX: 5, clientY: 100 }])
    dispatchTouch('touchmove', [{ clientX: 150, clientY: 105 }]) // base -288 + 145 = -143

    expect(panel.style.transform).toContain('translate3d(-143px')
  })

  it('closes when dragged back out while open', () => {
    const { onOpenChange } = renderDrawer(true)

    dispatchTouch('touchstart', [{ clientX: 200, clientY: 100 }])
    dispatchTouch('touchmove', [{ clientX: 30, clientY: 110 }]) // -170px, past -144
    dispatchTouch('touchend', [])

    expect(onOpenChange).toHaveBeenCalledWith(false)
  })
})
