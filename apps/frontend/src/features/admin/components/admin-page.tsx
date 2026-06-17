import { Link, type LinkProps } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '@/lib/components/ui/button'

interface AdminPageProps {
  title: string
  description?: string
  backTo?: { to: LinkProps['to']; params?: LinkProps['params']; label: string }
  actions?: ReactNode
  children: ReactNode
  /** Span the full page width instead of the default `max-w-4xl` container. */
  fullWidth?: boolean
}

export function AdminPage({
  title,
  description,
  backTo,
  actions,
  children,
  fullWidth,
}: AdminPageProps) {
  return (
    <div
      className={
        fullWidth ? 'w-full px-6 py-6 space-y-6' : 'container mx-auto max-w-4xl px-6 py-6 space-y-6'
      }
    >
      {backTo && (
        <Button variant="ghost" size="sm" asChild>
          <Link to={backTo.to} params={backTo.params}>
            <ArrowLeft className="mr-2 h-4 w-4" />
            {backTo.label}
          </Link>
        </Button>
      )}
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <h1 className="text-2xl font-semibold">{title}</h1>
          {description && <p className="text-sm text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="shrink-0">{actions}</div>}
      </div>
      {children}
    </div>
  )
}
