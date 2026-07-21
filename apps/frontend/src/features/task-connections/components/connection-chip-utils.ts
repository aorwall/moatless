import { Link2, MessageSquare, Send, Webhook } from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { Github } from '@/lib/components/ui/brand-icons'
import { LinearIcon } from './linear-icon'

type IconComponent = ComponentType<SVGProps<SVGSVGElement>>

export function adapterIcon(adapterKind: string): IconComponent {
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

export function statusBadgeClass(stateKind: string | null | undefined): string {
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

export function formatTimestamp(value: string | null | undefined): string {
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
export const ROW = 'flex h-7 items-center'
