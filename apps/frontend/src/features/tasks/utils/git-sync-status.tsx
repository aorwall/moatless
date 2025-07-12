import type { GitSyncStatus } from '@moatless/shared/types/api'
import { Check, ChevronDown, ChevronUp, FileWarning, GitBranch, XCircle } from 'lucide-react'

export interface GitSyncStatusConfig {
  icon: React.ReactNode
  color: string
  tooltip: string
  bgColor: string
}

export function getGitSyncStatusConfig(status: GitSyncStatus): GitSyncStatusConfig {
  switch (status) {
    case 'synced':
      return {
        icon: <Check className="h-3.5 w-3.5" />,
        color: 'text-green-600',
        bgColor: 'bg-green-100',
        tooltip: 'In sync with origin',
      }
    case 'ahead':
      return {
        icon: <ChevronUp className="h-3.5 w-3.5" />,
        color: 'text-blue-600',
        bgColor: 'bg-blue-100',
        tooltip: 'Ahead of origin',
      }
    case 'behind':
      return {
        icon: <ChevronDown className="h-3.5 w-3.5" />,
        color: 'text-yellow-600',
        bgColor: 'bg-yellow-100',
        tooltip: 'Behind origin',
      }
    case 'diverged':
      return {
        icon: <GitBranch className="h-3.5 w-3.5" />,
        color: 'text-orange-600',
        bgColor: 'bg-orange-100',
        tooltip: 'Diverged from origin',
      }
    case 'uncommitted_changes':
      return {
        icon: <FileWarning className="h-3.5 w-3.5" />,
        color: 'text-purple-600',
        bgColor: 'bg-purple-100',
        tooltip: 'Uncommitted changes',
      }
    default:
      return {
        icon: <XCircle className="h-3.5 w-3.5" />,
        color: 'text-gray-400',
        bgColor: 'bg-gray-100',
        tooltip: 'Unable to determine sync status',
      }
  }
}

export function formatGitSyncDetails(info: {
  commitsAhead?: number
  commitsBehind?: number
  hasUncommittedChanges?: boolean
}): string {
  const parts: string[] = []

  if (info.commitsAhead && info.commitsAhead > 0) {
    parts.push(`${info.commitsAhead} commit${info.commitsAhead > 1 ? 's' : ''} ahead`)
  }

  if (info.commitsBehind && info.commitsBehind > 0) {
    parts.push(`${info.commitsBehind} commit${info.commitsBehind > 1 ? 's' : ''} behind`)
  }

  if (info.hasUncommittedChanges) {
    parts.push('uncommitted changes')
  }

  return parts.join(', ')
}
