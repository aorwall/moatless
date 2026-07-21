import type { TaskConnectionRow } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { Badge } from '@/lib/components/ui/badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { cn } from '@/lib/utils'
import { ConnectionChip } from './connection-chip'
import { formatTimestamp, ROW, statusBadgeClass } from './connection-chip-utils'

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
