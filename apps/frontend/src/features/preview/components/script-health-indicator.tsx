import { AlertCircle, CheckCircle2, XCircle } from 'lucide-react'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { cn } from '@/lib/utils'
import type { ScriptHealthStatus } from '../types'

interface ScriptHealthIndicatorProps {
  status: ScriptHealthStatus
  className?: string
}

export function ScriptHealthIndicator({ status, className }: ScriptHealthIndicatorProps) {
  const getStatusInfo = () => {
    switch (status) {
      case 'not-injected':
        return {
          icon: XCircle,
          color: 'text-muted-foreground',
          label: 'Script Not Injected',
          description: 'Inspector script has not been injected into the preview',
        }
      case 'injected-not-reachable':
        return {
          icon: AlertCircle,
          color: 'text-yellow-500',
          label: 'Script Injected (Not Reachable)',
          description: 'Inspector script was injected but is not responding to messages',
        }
      case 'injected-reachable':
        return {
          icon: CheckCircle2,
          color: 'text-green-500',
          label: 'Script Active',
          description: 'Inspector script is injected and actively communicating',
        }
    }
  }

  const { icon: Icon, color, label, description } = getStatusInfo()

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div className={cn('flex items-center gap-1.5', className)}>
          <Icon className={cn('h-4 w-4', color)} />
          <span className="text-xs text-muted-foreground">Inspector</span>
        </div>
      </TooltipTrigger>
      <TooltipContent>
        <div className="space-y-1">
          <p className="font-medium">{label}</p>
          <p className="text-xs text-muted-foreground">{description}</p>
        </div>
      </TooltipContent>
    </Tooltip>
  )
}