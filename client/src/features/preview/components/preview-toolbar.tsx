import { ExternalLink, MousePointer, RotateCcw } from 'lucide-react'
import { type KeyboardEvent, useCallback, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { usePreviewStore } from '../stores/preview-store'

interface PreviewToolbarProps {
  onReload: () => void
}

export function PreviewToolbar({ onReload }: PreviewToolbarProps) {
  const { config, currentPath, isInspectorActive, setCurrentPath, toggleInspector } =
    usePreviewStore()

  const [pathInput, setPathInput] = useState(currentPath)

  const handlePathSubmit = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        let path = pathInput.trim()
        if (!path.startsWith('/')) {
          path = `/${path}`
        }
        setCurrentPath(path)
      }
    },
    [pathInput, setCurrentPath],
  )

  const handleOpenInNewTab = useCallback(() => {
    const fullUrl = `${config.url}${currentPath}`
    window.open(fullUrl, '_blank', 'noopener,noreferrer')
  }, [config.url, currentPath])

  return (
    <div className="flex items-center gap-2 p-2 border-b bg-muted/30">
      {/* Navigation Controls */}
      <div className="flex items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" onClick={onReload} className="h-8 w-8">
              <RotateCcw className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Reload Preview</TooltipContent>
        </Tooltip>
      </div>

      {/* Address Bar */}
      <div className="flex-1 flex items-center gap-2">
        <span className="text-sm text-muted-foreground whitespace-nowrap">{config.url}</span>
        <Input
          type="text"
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          onKeyDown={handlePathSubmit}
          placeholder="Enter path (e.g., /about)"
          className="flex-1 h-8 text-sm"
        />
      </div>

      {/* Tool Controls */}
      <div className="flex items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={isInspectorActive ? 'default' : 'ghost'}
              size="icon"
              onClick={toggleInspector}
              className="h-8 w-8"
            >
              <MousePointer className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Toggle Element Inspector</TooltipContent>
        </Tooltip>

        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" onClick={handleOpenInNewTab} className="h-8 w-8">
              <ExternalLink className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Open in New Tab</TooltipContent>
        </Tooltip>
      </div>
    </div>
  )
}
