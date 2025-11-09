import { Monitor, Smartphone, Tablet } from 'lucide-react'
import type { ViewportMode } from '@/features/tasks/contexts/task-ui-provider'
import { useViewportMode } from '@/features/tasks/contexts/task-ui-provider'
import { Button } from '@/lib/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'

// Helper function to get the next viewport mode in cycle
function getNextMode(currentMode: ViewportMode): ViewportMode {
  switch (currentMode) {
    case 'desktop':
      return 'mobile'
    case 'mobile':
      return 'tablet'
    case 'tablet':
      return 'desktop'
  }
}

// Helper function to get icon for current mode
function getIcon(mode: ViewportMode) {
  switch (mode) {
    case 'desktop':
      return Monitor
    case 'mobile':
      return Smartphone
    case 'tablet':
      return Tablet
  }
}

// Helper function to get tooltip text for current mode
function getTooltipText(mode: ViewportMode): string {
  switch (mode) {
    case 'desktop':
      return 'Desktop (Full Width)'
    case 'mobile':
      return 'Mobile (375px)'
    case 'tablet':
      return 'Tablet (768px)'
  }
}

export function ViewportToggle() {
  const { viewportMode, setViewportMode } = useViewportMode()

  const handleClick = () => {
    const nextMode = getNextMode(viewportMode)
    setViewportMode(nextMode)
  }

  const Icon = getIcon(viewportMode)
  const tooltipText = getTooltipText(viewportMode)

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          onClick={handleClick}
          className="h-8 w-8"
          aria-label={`Current viewport: ${viewportMode}`}
        >
          <Icon className="h-4 w-4" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tooltipText}</TooltipContent>
    </Tooltip>
  )
}
