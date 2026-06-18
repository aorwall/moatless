import type { Connection, TaskConnectionRow } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { ExternalLink, Link2, MessageSquare, Send, Webhook } from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { Badge } from '@/lib/components/ui/badge'
import { Github } from '@/lib/components/ui/brand-icons'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { cn } from '@/lib/utils'

type IconComponent = ComponentType<SVGProps<SVGSVGElement>>

/** Minimal Linear glyph — lucide has no Linear icon. */
function LinearIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 100 100" fill="currentColor" aria-hidden="true" {...props}>
      <path d="M1.2 61.5a1 1 0 0 1 1.7-.5l36 36a1 1 0 0 1-.5 1.7A50 50 0 0 1 1.2 61.5ZM.1 47.6a1 1 0 0 0 .3.8l51.2 51.2a1 1 0 0 0 .8.3 50.2 50.2 0 0 0 9-1.6 1 1 0 0 0 .5-1.7L3.4 38.1a1 1 0 0 0-1.7.5 50.2 50.2 0 0 0-1.6 9ZM6.3 25.9a1 1 0 0 0 .2 1.1l66.5 66.5a1 1 0 0 0 1.1.2c2.4-1.1 4.7-2.4 6.8-3.9a1 1 0 0 0 .1-1.5L11.7 18.9a1 1 0 0 0-1.5.1c-1.5 2.1-2.8 4.4-3.9 6.9ZM18.4 11.3a1 1 0 0 1-.1-1.4A50 50 0 0 1 90.1 81.7a1 1 0 0 1-1.4-.1L18.4 11.3Z" />
    </svg>
  )
}

function adapterIcon(adapterKind: string): IconComponent {
  switch (adapterKind) {
    case 'github':
    case 'github_pr':
      return Github
    case 'slack':
      return MessageSquare
    case 'linear':
      return LinearIcon
    case 'telegram':
      return Send
    case 'webhook':
      return Webhook
    default:
      return Link2
  }
}

/**
 * Color-coded badge classes keyed by normalized status bucket. Greens for
 * done (merged/completed), blues for in-flight (open/started), grays for
 * terminal-not-done (closed/canceled), amber for not-yet (draft/unstarted).
 */
function statusBadgeClass(stateKind: string | null | undefined): string {
  switch (stateKind) {
    case 'merged':
    case 'completed':
      return 'border-transparent bg-emerald-500/15 text-emerald-600 dark:text-emerald-400'
    case 'open':
    case 'started':
      return 'border-transparent bg-blue-500/15 text-blue-600 dark:text-blue-400'
    case 'closed':
    case 'canceled':
      return 'border-transparent bg-muted text-muted-foreground'
    case 'draft':
    case 'unstarted':
    case 'backlog':
    case 'triage':
      return 'border-transparent bg-amber-500/15 text-amber-600 dark:text-amber-400'
    default:
      return 'border-transparent bg-muted text-muted-foreground'
  }
}

function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return value
  }
  return date.toLocaleString()
}

/** Fixed row height so per-connection items stay aligned across columns. */
const ROW = 'flex h-7 items-center'

function ConnectionChip({ connection }: { connection: Connection }) {
  const Icon = adapterIcon(connection.adapterKind)
  // A deactivated connection is kept for history but no longer routes inbound
  // events (the task closed). Dim it and flag it so admins can tell at a glance.
  const inactive = connection.active === false
  return (
    <div className={cn(ROW, 'gap-1.5', inactive && 'opacity-50')}>
      <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
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

export function TaskConnectionsTable({ rows }: { rows: TaskConnectionRow[] }) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Task</TableHead>
          <TableHead>Repository</TableHead>
          <TableHead>Connections</TableHead>
          <TableHead>Status</TableHead>
          <TableHead className="text-right">Messages</TableHead>
          <TableHead>Last activity</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((row) => (
          <TableRow key={row.taskId} className="align-top">
            <TableCell className="py-3">
              <Link
                to="/tasks/$task-id"
                params={{ 'task-id': row.taskId }}
                className="font-medium hover:underline"
              >
                <span className="block max-w-[24rem] truncate" title={row.title}>
                  {row.title}
                </span>
              </Link>
              {row.owner && <div className="text-xs text-muted-foreground">{row.owner}</div>}
            </TableCell>
            <TableCell className="py-3 text-sm text-muted-foreground">
              {row.repositoryName ?? '-'}
            </TableCell>

            {row.connections.length === 0 ? (
              <>
                <TableCell className="py-3 text-sm text-muted-foreground">-</TableCell>
                <TableCell className="py-3" />
                <TableCell className="py-3" />
                <TableCell className="py-3" />
              </>
            ) : (
              <>
                <TableCell className="py-3">
                  <div className="space-y-1">
                    {row.connections.map((c) => (
                      <ConnectionChip key={c.bindingId} connection={c} />
                    ))}
                  </div>
                </TableCell>
                <TableCell className="py-3">
                  <div className="space-y-1">
                    {row.connections.map((c) => (
                      <div key={c.bindingId} className={ROW}>
                        {c.status ? (
                          <Badge className={cn('font-normal', statusBadgeClass(c.stateKind))}>
                            {c.status}
                          </Badge>
                        ) : (
                          <span className="text-xs text-muted-foreground">unknown</span>
                        )}
                      </div>
                    ))}
                  </div>
                </TableCell>
                <TableCell className="py-3 text-right tabular-nums">
                  <div className="space-y-1">
                    {row.connections.map((c) => (
                      <div key={c.bindingId} className={cn(ROW, 'justify-end gap-2 text-xs')}>
                        <span className="text-muted-foreground">↓{c.incoming}</span>
                        <span className="text-muted-foreground">↑{c.outgoing}</span>
                      </div>
                    ))}
                  </div>
                </TableCell>
                <TableCell className="py-3">
                  <div className="space-y-1">
                    {row.connections.map((c) => (
                      <div key={c.bindingId} className={cn(ROW, 'text-xs text-muted-foreground')}>
                        {formatTimestamp(c.lastEventAt)}
                      </div>
                    ))}
                  </div>
                </TableCell>
              </>
            )}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
