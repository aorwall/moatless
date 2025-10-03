import { AlertCircle, CheckCircle, Info, Loader2, X, XCircle } from 'lucide-react'
import { useState } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'

export interface ChatStatusBannerProps {
  variant: 'info' | 'warning' | 'error' | 'success' | 'loading'
  message: string
  description?: string
  action?: {
    label: string
    onClick: () => void
  }
  dismissible?: boolean
  onDismiss?: () => void
  className?: string
}

const variantConfig = {
  info: {
    icon: Info,
    alertVariant: 'default' as const,
    iconClass: 'text-blue-500',
  },
  warning: {
    icon: AlertCircle,
    alertVariant: 'default' as const,
    iconClass: 'text-yellow-500',
  },
  error: {
    icon: XCircle,
    alertVariant: 'destructive' as const,
    iconClass: 'text-destructive',
  },
  success: {
    icon: CheckCircle,
    alertVariant: 'default' as const,
    iconClass: 'text-green-500',
  },
  loading: {
    icon: Loader2,
    alertVariant: 'default' as const,
    iconClass: 'text-blue-500 animate-spin',
  },
}

export function ChatStatusBanner({
  variant,
  message,
  description,
  action,
  dismissible = false,
  onDismiss,
  className,
}: ChatStatusBannerProps) {
  const [isDismissed, setIsDismissed] = useState(false)

  const config = variantConfig[variant]
  const Icon = config.icon

  const handleDismiss = () => {
    setIsDismissed(true)
    onDismiss?.()
  }

  if (isDismissed) {
    return null
  }

  return (
    <div className={cn('flex-shrink-0 px-4 pb-2', className)}>
      <Alert variant={config.alertVariant} className="relative">
        <Icon className={cn('h-4 w-4', config.iconClass)} />
        <AlertTitle className="flex items-center justify-between">
          {message}
          {dismissible && (
            <Button
              variant="ghost"
              size="sm"
              onClick={handleDismiss}
              className="h-auto p-1 hover:bg-transparent"
            >
              <X className="h-3 w-3" />
            </Button>
          )}
        </AlertTitle>
        {description && <AlertDescription>{description}</AlertDescription>}
        {action && (
          <div className="col-start-2 mt-2">
            <Button variant="outline" size="sm" onClick={action.onClick}>
              {action.label}
            </Button>
          </div>
        )}
      </Alert>
    </div>
  )
}
