import type { ReactNode } from 'react'
import type { ViewportMode } from '@/features/tasks/hooks/use-task-view'
import { cn } from '@/lib/utils'

interface DeviceFrameProps {
  mode: ViewportMode
  children: ReactNode
  className?: string
}

const VIEWPORT_WIDTHS = {
  mobile: 375,
  tablet: 768,
  desktop: '100%',
} as const

const VIEWPORT_HEIGHTS = {
  mobile: 667, // iPhone 8
  tablet: 1024, // iPad portrait
  desktop: '100%',
} as const

export function DeviceFrame({ mode, children, className }: DeviceFrameProps) {
  // Desktop mode - no frame, full width and height
  if (mode === 'desktop') {
    return <div className={cn('w-full h-full', className)}>{children}</div>
  }

  // Mobile/Tablet mode - centered with device frame and height constraint
  const width = VIEWPORT_WIDTHS[mode]
  const height = VIEWPORT_HEIGHTS[mode]

  return (
    <div className={cn('w-full h-full flex items-center justify-center bg-muted/30', className)}>
      <div
        className="bg-background border border-border shadow-lg rounded-lg overflow-hidden"
        style={{
          width: '100%',
          height: '100%',
          maxWidth: `${width}px`,
          maxHeight: `${height}px`,
        }}
      >
        {children}
      </div>
    </div>
  )
}
