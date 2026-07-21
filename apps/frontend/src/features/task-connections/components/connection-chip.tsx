import type { Connection } from '@moatless/api-client'
import { ExternalLink } from 'lucide-react'
import { createElement } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { cn } from '@/lib/utils'
import { adapterIcon, ROW } from './connection-chip-utils'

export function ConnectionChip({ connection }: { connection: Connection }) {
  const Icon = adapterIcon(connection.adapterKind)
  // A deactivated connection is kept for history but no longer routes inbound
  // events (the task closed). Dim it and flag it so admins can tell at a glance.
  const inactive = connection.active === false
  return (
    <div className={cn(ROW, 'gap-1.5', inactive && 'opacity-50')}>
      {createElement(Icon, { className: 'h-4 w-4 shrink-0 text-muted-foreground' })}
      {connection.url ? (
        <a
          href={connection.url}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 font-medium hover:underline"
        >
          <span className="truncate">{connection.label}</span>
          <ExternalLink className="h-3 w-3 shrink-0 opacity-60" />
        </a>
      ) : (
        <span className="truncate font-medium">{connection.label}</span>
      )}
      {inactive && (
        <Badge
          variant="outline"
          className="shrink-0 px-1 py-0 text-[10px] font-normal text-muted-foreground"
        >
          inactive
        </Badge>
      )}
    </div>
  )
}
