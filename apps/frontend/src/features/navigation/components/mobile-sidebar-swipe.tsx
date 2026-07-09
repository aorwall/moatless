import { useCallback } from 'react'
import { useSidebar } from '@/lib/components/ui/sidebar'
import { useEdgeSwipe } from '@/lib/hooks/use-edge-swipe'

/**
 * Opens the task sidebar when the user swipes in from the left edge on a touch
 * device, instead of the browser's back-navigation gesture. Renders nothing;
 * it only registers the gesture handler. Must be used within a
 * `SidebarProvider`.
 */
export function MobileSidebarSwipe() {
  const { isMobile, openMobile, setOpenMobile } = useSidebar()

  const openSidebar = useCallback(() => {
    setOpenMobile(true)
  }, [setOpenMobile])

  useEdgeSwipe({
    enabled: isMobile && !openMobile,
    onSwipeRight: openSidebar,
  })

  return null
}
