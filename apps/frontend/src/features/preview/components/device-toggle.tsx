import { Monitor, Smartphone, Tablet } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { type DeviceMode, usePreviewDevice } from '../hooks/use-preview-view'

// Helper function to get the next device mode in cycle
function getNextMode(currentMode: DeviceMode): DeviceMode {
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
function getIcon(mode: DeviceMode) {
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
function getTooltipText(mode: DeviceMode): string {
  switch (mode) {
    case 'desktop':
      return 'Desktop (Full Width)'
    case 'mobile':
      return 'Mobile (375px)'
    case 'tablet':
      return 'Tablet (768px)'
  }
}

export function DeviceToggle() {
  const { device: deviceMode, setDevice: setDeviceMode } = usePreviewDevice()

  const handleClick = () => {
    const nextMode = getNextMode(deviceMode)
    setDeviceMode(nextMode)
  }

  const Icon = getIcon(deviceMode)
  const tooltipText = getTooltipText(deviceMode)

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          onClick={handleClick}
          className="h-8 w-8"
          aria-label={`Current device: ${deviceMode}`}
        >
          <Icon className="h-4 w-4" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tooltipText}</TooltipContent>
    </Tooltip>
  )
}
